//! HTTP 处理器：解析请求 -> 调用 PHP 侧回调 -> 返回响应。

use crate::runtime::WorkerConfig;
use ext_php_rs::types::ZendCallable;
use socket2::{Domain, Socket, Type};
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// 一次性封装 PHP 侧注册的 4 个回调（配合 leak_callable 生成 'static 引用）。
#[derive(Copy, Clone)]
struct HandlerBundle {
    http: &'static Option<&'static ZendCallable<'static>>,
    ws_open: &'static Option<&'static ZendCallable<'static>>,
    ws_message: &'static Option<&'static ZendCallable<'static>>,
    ws_close: &'static Option<&'static ZendCallable<'static>>,
}

// SAFETY:
//   `ZendCallable` 内部只是指向 PHP 函数表的指针，进程生命周期内
//   地址不变。我们通过 `runtime::PHP_CALL_LOCK` 全局互斥锁保证同一
//   时刻只有一个线程进入 PHP 解释器，因此跨线程传递 `HandlerBundle`
//   是安全的。没有这个 `Send` impl，`tokio::spawn` 无法用它（它
//   要求 `Future: Send`）。
unsafe impl Send for HandlerBundle {}
unsafe impl Sync for HandlerBundle {}

/// 在主循环启动前，对端口做一次快速的占用检测。
/// 如果被占用则打印清晰的命令提示并返回 Err，从而避免父进程进入
/// 无限重启循环。
pub fn probe_port(host: &str, port: u16) -> Result<(), String> {
    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;
    let domain = if addr.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };
    let socket = Socket::new(domain, Type::STREAM, Some(socket2::Protocol::TCP))
        .map_err(|e| format!("socket: {}", e))?;
    let _ = socket.set_reuse_address(true);
    let _ = socket.set_reuse_port(true);
    match socket.bind(&addr.into()) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => Err(format!(
            "[lychee-worker] Port {}:{} is already in use, please change the port",
            host, port,
        )),
        Err(e) => Err(format!("bind {}:{}: {}", host, port, e)),
    }
}

/// 以 SO_REUSEADDR + SO_REUSEPORT 的方式绑定端口，允许多个
/// HTTP 子进程共享监听端口，从而由内核在进程间做负载均衡。
fn bind_with_reuse(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let domain = if addr.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };
    let socket = Socket::new(domain, Type::STREAM, Some(socket2::Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    let std_listener: StdTcpListener = socket.into();
    TcpListener::from_std(std_listener)
}

pub async fn serve<'a>(
    host: String,
    port: u16,
    http_handler: Option<&'a ZendCallable<'a>>,
    ws_open_handler: Option<&'a ZendCallable<'a>>,
    ws_message_handler: Option<&'a ZendCallable<'a>>,
    ws_close_handler: Option<&'a ZendCallable<'a>>,
    cfg: &'a WorkerConfig,
) -> std::io::Result<()> {
    let addr: SocketAddr =
        format!("{}:{}", host, port)
            .parse()
            .map_err(|e: std::net::AddrParseError| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string())
            })?;
    let listener = bind_with_reuse(addr)?;

    // 将回调引用泄露为 'static，以便在 tokio spawn_local 的任务中使用
    let handlers = HandlerBundle {
        http: leak_callable(http_handler),
        ws_open: leak_callable(ws_open_handler),
        ws_message: leak_callable(ws_message_handler),
        ws_close: leak_callable(ws_close_handler),
    };

    // 从配置中提取运行参数
    let request_timeout = Duration::from_secs(cfg.request_timeout_sec);
    let max_connections = cfg.max_connections as i64;
    let header_max = cfg.header_max_bytes;
    let body_max = cfg.body_max_bytes;
    let ping_interval = cfg.ping_interval_sec;
    let ping_timeout = cfg.ping_timeout_sec;

    // 使用普通 tokio::spawn：`HandlerBundle` 现在是 Send，因此每个连接任务
    // 可以被调度到任何 worker 线程上。当某个任务调用 `run_php_blocking`
    // 被 PHP 阻塞时，tokio 会把其他 pending task 迁移到另一条 worker 线程，
    // 保证该子进程的 WebSocket 心跳和其他 HTTP 请求不被卡住。
    loop {
        match listener.accept().await {
            Ok((stream, _remote)) => {
                // 连接数上限检查：超限直接 503 关闭，避免耗尽资源
                let current = crate::runtime::ACTIVE_HTTP_CONNS.fetch_add(1, Ordering::SeqCst) + 1;
                if current > max_connections {
                    crate::runtime::ACTIVE_HTTP_CONNS.fetch_sub(1, Ordering::SeqCst);
                    tokio::spawn(async move {
                        let _ = reject_service_unavailable(stream).await;
                    });
                    continue;
                }

                tokio::spawn(async move {
                    let result = handle_connection(
                        stream,
                        handlers,
                        request_timeout,
                        header_max,
                        body_max,
                        ping_interval,
                        ping_timeout,
                    )
                    .await;
                    // 连接结束递减计数
                    crate::runtime::ACTIVE_HTTP_CONNS.fetch_sub(1, Ordering::SeqCst);
                    let _ = result;
                });
            }
            Err(e) => {
                eprintln!("[lychee-worker] accept error: {}", e);
            }
        }
    }
}

/// 连接数上限时快速返回 503
async fn reject_service_unavailable(mut stream: TcpStream) -> std::io::Result<()> {
    let body =
        b"<html><body><h1>503 Service Unavailable</h1><p>Too many connections.</p></body></html>";
    let response = format!(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = tokio::time::timeout(Duration::from_secs(5), async move {
        stream.write_all(response.as_bytes()).await?;
        stream.write_all(body).await?;
        stream.flush().await
    })
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "write timeout"))?;
    Ok(())
}

fn leak_callable<'a>(
    cb: Option<&'a ZendCallable<'a>>,
) -> &'static Option<&'static ZendCallable<'static>> {
    let boxed: Box<Option<&'static ZendCallable<'static>>> = Box::new(unsafe {
        std::mem::transmute::<Option<&'a ZendCallable<'a>>, Option<&'static ZendCallable<'static>>>(
            cb,
        )
    });
    Box::leak(boxed)
}

async fn handle_connection(
    mut stream: TcpStream,
    handlers: HandlerBundle,
    request_timeout: Duration,
    header_max: usize,
    body_max: usize,
    ping_interval_sec: u64,
    ping_timeout_sec: u64,
) -> std::io::Result<()> {
    // 外循环：支持 HTTP keep-alive，一个 TCP 连接处理多个请求
    loop {
        // 1) 读取 header（带超时保护）
        let buf_full =
            match tokio::time::timeout(request_timeout, read_http_headers(&mut stream, header_max))
                .await
            {
                Ok(Ok(v)) => {
                    if v.is_empty() {
                        return Ok(()); // EOF：客户端关闭
                    }
                    v
                }
                Ok(Err(_)) => {
                    let _ = write_simple_response(&mut stream, 400, "Bad Request", "").await;
                    return Ok(());
                }
                Err(_) => {
                    let _ = write_simple_response(&mut stream, 408, "Request Timeout", "").await;
                    return Ok(());
                }
            };

        // 2) WebSocket 升级判定
        let head_preview = &buf_full[..buf_full.len().min(1024)];
        let head_str = String::from_utf8_lossy(head_preview).to_string();
        if head_str.to_lowercase().contains("upgrade: websocket") {
            return crate::websocket::handle_upgrade(
                stream,
                &buf_full,
                crate::websocket::WsHandlerBundle {
                    on_open: *handlers.ws_open,
                    on_message: *handlers.ws_message,
                    on_close: *handlers.ws_close,
                },
                ping_interval_sec,
                ping_timeout_sec,
            )
            .await;
        }

        // 3) 解析 header 结束位置
        let header_end = match buf_full.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(p) => p + 4,
            None => {
                let _ = write_simple_response(&mut stream, 400, "Bad Request", "").await;
                return Ok(());
            }
        };
        let header_bytes = &buf_full[..header_end];

        // 4) 判断是否 keep-alive
        let keep_alive = wants_keep_alive(header_bytes);

        // 5) 读取 body（带超时和大小限制）
        let body = match tokio::time::timeout(
            request_timeout,
            read_body(header_bytes, &mut stream, &buf_full[header_end..], body_max),
        )
        .await
        {
            Ok(Ok(b)) => b,
            Ok(Err(_)) => {
                let _ = write_simple_response(&mut stream, 413, "Payload Too Large", "").await;
                return Ok(());
            }
            Err(_) => {
                let _ = write_simple_response(&mut stream, 408, "Request Timeout", "").await;
                return Ok(());
            }
        };

        // 6) 解析 method/path/headers
        let raw_headers = extract_headers_text(header_bytes);
        let (method, path) = parse_method_path(header_bytes);

        // 6a) 准备 SSE 写入通道：dup socket fd 供 PHP 扩展同步写入
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            unsafe { crate::sse::set_stream_fd(stream.as_raw_fd()) };
        }

        // 7) 调用 PHP 回调。
        //    - `run_php_blocking` 把当前线程临时退出 reactor，
        //      tokio 会在另一条 worker 线程上继续处理其他连接。
        //    - `request_timeout` 外层保护：即使 PHP 内部无法被强制
        //      中断，也能在超时后给客户端返回 504 Gateway Timeout，
        //      同时释放本线程让其继续服务其他连接。
        let http_handler = *handlers.http;
        let method_c = method.clone();
        let path_c = path.clone();
        let response = match tokio::time::timeout(
            request_timeout,
            std::future::ready(crate::runtime::run_php_blocking(move || {
                crate::php_api::call_on_http(http_handler, &method_c, &path_c, &raw_headers, &body)
            })),
        )
        .await
        {
            Ok(resp) => resp,
            Err(_) => {
                eprintln!(
                    "[lychee-worker] PHP handler timeout (>{:.0}s) for {} {}",
                    request_timeout.as_secs_f64(),
                    method,
                    path
                );
                "HTTP/1.1 504 Gateway Timeout\r\n\
                 Content-Type: text/plain; charset=utf-8\r\n\
                 Content-Length: 19\r\n\
                 Connection: close\r\n\
                 \r\n\
                 Gateway Timeout\n"
                    .to_string()
            }
        };

        // 8) 检查是否使用 SSE：使用了则跳过正常响应写入
        let sse_used = crate::sse::was_sse_started();
        crate::sse::clear_stream();

        // 9) 请求计数 +1
        crate::runtime::REQ_COUNT.fetch_add(1, Ordering::SeqCst);

        // 10) SSE 模式下 PHP 已直接写数据，跳过；否则写响应
        if !sse_used {
            match tokio::time::timeout(request_timeout, async {
                stream.write_all(response.as_bytes()).await?;
                stream.flush().await
            })
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "response write timeout",
                    ));
                }
            }
        }

        // 11) keep-alive 决策：SSE 后直接关闭；其他按 Connection 头
        if sse_used || !keep_alive {
            return Ok(());
        }
        // 继续循环，等待下一个请求
    }
}

/// 从 stream 读取 HTTP header，直到遇到 \r\n\r\n 或达到 header_max
async fn read_http_headers(stream: &mut TcpStream, header_max: usize) -> std::io::Result<Vec<u8>> {
    let mut buf: Vec<u8> = vec![0u8; 32768.min(header_max + 4096)];
    let mut total: usize = 0;
    loop {
        if total >= buf.len() {
            if buf.len() >= header_max {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "header too large",
                ));
            }
            let new_cap = (buf.len() * 2).min(header_max + 4096);
            buf.resize(new_cap, 0);
        }
        let n = stream.read(&mut buf[total..]).await?;
        if n == 0 {
            if total == 0 {
                return Ok(Vec::new());
            }
            break;
        }
        total += n;
        if total >= 4 {
            let search_end = total.min(header_max);
            if buf[..search_end].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        if total >= header_max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "header too large",
            ));
        }
    }
    buf.truncate(total);
    Ok(buf)
}

/// 判断请求是否希望保持连接（HTTP/1.1 默认 keep-alive，显式 Connection: close 则关闭）
fn wants_keep_alive(header_bytes: &[u8]) -> bool {
    let first_line = header_bytes
        .split(|b| *b == b'\n')
        .next()
        .map(|l| String::from_utf8_lossy(l).to_string())
        .unwrap_or_default();
    let is_http_1_1 = first_line.to_ascii_lowercase().contains("http/1.1");

    for line in header_bytes.split(|b| *b == b'\n') {
        let line_str = String::from_utf8_lossy(line);
        let trimmed = line_str.trim_end_matches('\r');
        if trimmed.to_ascii_lowercase().starts_with("connection:") {
            let value = trimmed["connection:".len()..].trim().to_ascii_lowercase();
            return value.contains("keep-alive") && !value.contains("close");
        }
    }
    is_http_1_1
}

/// 写一个简单的错误响应
async fn write_simple_response(
    stream: &mut TcpStream,
    status_code: u16,
    status_text: &str,
    _body_html: &str,
) -> std::io::Result<()> {
    let body = format!(
        "<html><body><h1>{} {}</h1></body></html>",
        status_code, status_text
    );
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_code,
        status_text,
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

/// 根据 header 中的 Content-Length 读取 body。Content-Length 超过 body_max
/// 直接返回错误（由上层返回 413）。
async fn read_body(
    header_bytes: &[u8],
    stream: &mut TcpStream,
    pre_body: &[u8],
    body_max: usize,
) -> std::io::Result<String> {
    let mut body_vec: Vec<u8> = pre_body.to_vec();

    let content_length = header_bytes
        .split(|b| *b == b'\n')
        .find_map(|line| {
            let line = std::str::from_utf8(line).ok()?;
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("content-length:") {
                return value.trim().parse::<usize>().ok();
            }
            None
        })
        .unwrap_or(0);

    if content_length == 0 {
        return Ok(String::from_utf8_lossy(&body_vec).to_string());
    }

    // Content-Length 超过上限直接拒绝（避免攻击者制造大内存分配）
    if content_length > body_max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "body too large",
        ));
    }

    while body_vec.len() < content_length {
        let mut tmp = vec![0u8; 4096];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        body_vec.extend_from_slice(&tmp[..n]);
        if body_vec.len() >= content_length {
            break;
        }
    }
    if body_vec.len() > content_length {
        body_vec.truncate(content_length);
    }

    Ok(String::from_utf8_lossy(&body_vec).to_string())
}

/// 从请求字节中提取 header 文本（不包含请求行，不包含末尾的 \r\n\r\n）。
/// 返回每行 "Name: Value" 用 \r\n 连接的字符串。
fn extract_headers_text(header_bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(header_bytes);
    let mut lines: Vec<&str> = text.split("\r\n").collect();
    // 去除第一行（请求行）
    if !lines.is_empty() {
        lines.remove(0);
    }
    // 去掉最后一个空元素（由末尾 \r\n\r\n 产生）
    while let Some(last) = lines.last() {
        if last.trim().is_empty() {
            lines.pop();
        } else {
            break;
        }
    }
    lines.join("\r\n")
}

fn parse_method_path(bytes: &[u8]) -> (String, String) {
    let first_line = bytes
        .split(|b| *b == b'\n')
        .next()
        .map(|l| String::from_utf8_lossy(l).to_string())
        .unwrap_or_default();
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let method = parts.first().copied().unwrap_or("GET").to_string();
    let path = parts.get(1).copied().unwrap_or("/").to_string();
    (method, path)
}
