//! HTTP 处理器：解析请求 -> 调用 PHP 侧回调 -> 返回响应。

use ext_php_rs::types::ZendCallable;
use socket2::{Domain, Socket, Type};
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// 在主循环启动前，对端口做一次快速的占用检测。
/// 如果被占用则打印清晰的命令提示并返回 Err，从而避免父进程进入
/// 无限重启循环。
pub fn probe_port(host: &str, port: u16) -> Result<(), String> {
    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;
    let domain = if addr.is_ipv6() { Domain::IPV6 } else { Domain::IPV4 };
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
    let domain = if addr.is_ipv6() { Domain::IPV6 } else { Domain::IPV4 };
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
) -> std::io::Result<()> {
    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e: std::net::AddrParseError| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string())
        })?;
    let listener = bind_with_reuse(addr)?;

    // 将回调引用泄露为 'static，以便在 tokio spawn_local 的任务中使用
    // 子进程在退出前会一直持有这些引用，因此是安全的
    let leaked_http = leak_callable(http_handler);
    let leaked_open = leak_callable(ws_open_handler);
    let leaked_msg = leak_callable(ws_message_handler);
    let leaked_close = leak_callable(ws_close_handler);

    let local = tokio::task::LocalSet::new();
    local.run_until(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _remote)) => {
                    tokio::task::spawn_local(async move {
                        let _ = handle_connection(
                            stream,
                            leaked_http,
                            leaked_open,
                            leaked_msg,
                            leaked_close,
                        ).await;
                    });
                }
                Err(e) => {
                    eprintln!("[lychee-worker] accept error: {}", e);
                }
            }
        }
    }).await;

    Ok(())
}

fn leak_callable<'a>(
    cb: Option<&'a ZendCallable<'a>>,
) -> &'static Option<&'static ZendCallable<'static>> {
    let boxed: Box<Option<&'static ZendCallable<'static>>> = Box::new(
        unsafe { std::mem::transmute::<Option<&'a ZendCallable<'a>>, Option<&'static ZendCallable<'static>>>(cb) },
    );
    Box::leak(boxed)
}

async fn handle_connection(
    mut stream: TcpStream,
    http_handler: &'static Option<&'static ZendCallable<'static>>,
    ws_open_handler: &'static Option<&'static ZendCallable<'static>>,
    ws_message_handler: &'static Option<&'static ZendCallable<'static>>,
    ws_close_handler: &'static Option<&'static ZendCallable<'static>>,
) -> std::io::Result<()> {
    let mut buf = vec![0u8; 32768];
    let mut total: usize = 0;
    loop {
        let n = stream.read(&mut buf[total..]).await?;
        if n == 0 {
            break;
        }
        total += n;
        if total >= 4 {
            let head = &buf[..total];
            if head.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        if total >= 1_048_576 {
            break;
        }
    }
    if total == 0 {
        return Ok(());
    }
    let head_str = String::from_utf8_lossy(&buf[..total.min(1024)]).to_string();
    if head_str.to_lowercase().contains("upgrade: websocket") {
        // HTTP 握手请求已经被读入 buf 中，不能让 tungstenite 的
        // from_raw_socket 再去 stream 上读取一次（会读到 EOF）。
        // 把已读取的 bytes 重新注入到 stream 之前，再交给 WebSocket
        // 处理循环。
        return crate::websocket::handle_upgrade(
            stream,
            &buf[..total],
            *ws_open_handler,
            *ws_message_handler,
            *ws_close_handler,
        )
        .await;
    }
    // 找到 header 结束位置
    let header_end = buf[..total]
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(total);
    let header_bytes = &buf[..header_end];

    // 读取请求体（根据 Content-Length）
    let body = read_body(header_bytes, &mut stream, &buf[header_end..total]).await?;

    // 提取 header 文本（不包含第一行请求行）
    let raw_headers = extract_headers_text(header_bytes);

    let (method, path) = parse_method_path(header_bytes);
    // PHP 回调现在返回完整的 HTTP 响应文本（含 header 部分）。
    // 直接写回给客户端。
    let response = crate::php_api::call_on_http(*http_handler, &method, &path, &raw_headers, &body);
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

/// 根据 header 中的 Content-Length（如有），继续从 stream 中读取剩余
/// body 字节。如果没有 Content-Length 则直接返回已读取的 pre_body。
async fn read_body(
    header_bytes: &[u8],
    stream: &mut TcpStream,
    pre_body: &[u8],
) -> std::io::Result<String> {
    let mut body_vec: Vec<u8> = pre_body.to_vec();

    // 从 header 文本中解析 Content-Length
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

    // 继续从 stream 中读取，直到获得足够的 body
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
    // 只取 Content-Length 指定的字节数
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
    while lines.last().map_or(false, |l| l.trim().is_empty()) {
        lines.pop();
        if lines.is_empty() {
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
    let method = parts.get(0).copied().unwrap_or("GET").to_string();
    let path = parts.get(1).copied().unwrap_or("/").to_string();
    (method, path)
}