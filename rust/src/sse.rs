//! Server-Sent Events (SSE) 支持。
//!
//! 设计思路：PHP 回调在 tokio runtime 内同步执行。为了让 PHP 能
//! 在回调内部向同一个 TCP 连接写入流式数据（而无需等待回调返回后
//! 再写），我们在调用 PHP 之前把 socket fd dup 一份，包装成
//! 阻塞的 `std::net::TcpStream` 存入 thread_local。PHP 通过
//! `lychee_worker_sse_start/send/end` 函数直接在这个阻塞流上写。
//!
//! 这种方式有几个优点：
//!   1. 不需要在 PHP 回调中桥接 async/await
//!   2. 不需要 `block_in_place`（对 `current_thread` runtime 友好）
//!   3. 两个 fd（tokio 的 + std 的）指向同一个 TCP 连接内核态，
//!      写入顺序由同一线程的执行顺序保证
//!
//! # 协议格式
//!   - 响应头：HTTP/1.1 200 OK + Content-Type: text/event-stream
//!   - Transfer-Encoding: chunked
//!   - 事件：`event: name\r\ndata: payload\r\n\r\n`
//!   - chunk：`<hex_len>\r\n<bytes>\r\n`，终止：`0\r\n\r\n`

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::net::TcpStream;

// 线程局部：当前 PHP 回调可写的阻塞 TCP 流（dup 自 tokio 流）。
// 在 `http.rs::handle_connection` 调用 PHP 前设置，PHP 返回后清除。
thread_local! {
    static ACTIVE_STREAM: RefCell<Option<TcpStream>> = RefCell::new(None);
    static SSE_STARTED: Cell<bool> = Cell::new(false);
}

/// 通过 Unix raw fd 创建一个阻塞的 TcpStream 并存入线程局部。
/// 仅在 `http.rs::handle_connection` 调用 PHP 回调前调用一次。
///
/// # 安全
/// 调用者必须保证 `raw_fd` 是一个有效的、连接状态的 socket 文件描述符。
/// 此函数内部会 `dup` 该 fd，所有权由新创建的 `TcpStream` 独占，
/// 调用结束后由 `clear_stream` 释放。
#[cfg(unix)]
pub(crate) unsafe fn set_stream_fd(raw_fd: std::os::unix::io::RawFd) {
    use std::os::unix::io::FromRawFd;
    let duped = unsafe { libc::dup(raw_fd) };
    if duped < 0 {
        return;
    }
    let stream = unsafe { TcpStream::from_raw_fd(duped) };
    // 设为阻塞模式（tokio 的原始 fd 是非阻塞的，dup 继承非阻塞属性）
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    ACTIVE_STREAM.with(|cell| {
        *cell.borrow_mut() = Some(stream);
    });
    SSE_STARTED.with(|cell| cell.set(false));
}

/// 非 Unix 平台的空实现（编译通过，但运行时不可用）。
#[cfg(not(unix))]
pub(crate) unsafe fn set_stream_fd(_raw_fd: i32) {
    // 在非 Unix 平台上，SSE 功能不可用；保持函数签名一致。
}

/// 清除并释放当前 SSE 流，由 PHP 回调返回后调用。
/// 注意：此处不调用 shutdown(SHUT_RDWR)，因为 dup 的 fd 与 tokio 侧共享同一
/// 同一个底层 socket；shutdown 会导致 tokio 无法写响应。直接 drop 掉流即可。
pub(crate) fn clear_stream() {
    ACTIVE_STREAM.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if let Some(mut stream) = borrow.take() {
            // 仅 flush 未写的字节（若有）；如果是 SSE 模式已有 end() 已写
            // 终止符，否则流在 drop 时释放 fd 自动关闭
            let _ = stream.flush();
        }
    });
    SSE_STARTED.with(|cell| cell.set(false));
}

/// 本次请求是否使用了 SSE（由 `lychee_worker_sse_start` 置位）。
pub(crate) fn was_sse_started() -> bool {
    SSE_STARTED.with(|cell| cell.get())
}

/// 写一个 chunk（chunked transfer encoding）。
/// 格式：`{hex_len}\r\n{data}\r\n`
fn write_chunked(data: &[u8]) -> bool {
    ACTIVE_STREAM.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(stream) = borrow.as_mut() else {
            return false;
        };
        let header = format!("{:x}\r\n", data.len());
        let r1 = stream.write_all(header.as_bytes());
        let r2 = stream.write_all(data);
        let r3 = stream.write_all(b"\r\n");
        r1.and(r2).and(r3).is_ok()
    })
}

/// 写 chunked 终止标记 `0\r\n\r\n`，然后 flush。
fn write_chunked_terminator() -> bool {
    ACTIVE_STREAM.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(stream) = borrow.as_mut() else {
            return false;
        };
        let _ = stream.write_all(b"0\r\n\r\n");
        stream.flush().is_ok()
    })
}

/// 直接写原始字节到活动流（不包 chunk，用于写 HTTP 响应头）。
fn write_raw(data: &[u8]) -> bool {
    ACTIVE_STREAM.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(stream) = borrow.as_mut() else {
            return false;
        };
        stream.write_all(data).is_ok()
    })
}

/// 格式化一条 SSE 事件消息（不含 chunked 编码外层）。
///
/// 输出：`event: <name>\r\ndata: <data>\r\n\r\n`
/// event 为空时省略 event 行（浏览器默认事件名为 "message"）。
pub fn format_event(event: &str, data: &str) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::with_capacity(event.len() + data.len() + 32);
    if !event.is_empty() {
        buf.extend_from_slice(b"event: ");
        // event 名含 \r/\n 时截断（避免注入额外字段）
        let clean: String = event
            .chars()
            .take_while(|c| *c != '\r' && *c != '\n')
            .collect();
        buf.extend_from_slice(clean.as_bytes());
        buf.extend_from_slice(b"\r\n");
    }
    // data 换行 → 多个 data: 前缀
    for line in data.split('\n') {
        buf.extend_from_slice(b"data: ");
        let clean: String = line.chars().take_while(|c| *c != '\r').collect();
        buf.extend_from_slice(clean.as_bytes());
        buf.extend_from_slice(b"\r\n");
    }
    buf.extend_from_slice(b"\r\n");
    buf
}

/// 启动 SSE：写 HTTP 响应头 + 一条 retry 提示 chunk。
pub fn start() -> bool {
    let has_stream = ACTIVE_STREAM.with(|cell| cell.borrow().is_some());
    if !has_stream {
        return false;
    }
    let headers = b"HTTP/1.1 200 OK\r\n\
                    Content-Type: text/event-stream\r\n\
                    Cache-Control: no-cache\r\n\
                    Connection: keep-alive\r\n\
                    Transfer-Encoding: chunked\r\n\
                    \r\n";
    if !write_raw(headers) {
        return false;
    }
    SSE_STARTED.with(|cell| cell.set(true));
    // 首条 chunk：提示浏览器断线后 3 秒重连
    write_chunked(b"retry: 3000\r\n\r\n")
}

/// 发送一条 SSE 事件（以 chunk 形式写入）。
pub fn send(event: String, data: String) -> bool {
    if !SSE_STARTED.with(|cell| cell.get()) {
        return false;
    }
    let bytes = format_event(&event, &data);
    write_chunked(&bytes)
}

/// 结束 SSE：写 chunked 终止标记并 flush。
pub fn end() -> bool {
    if !SSE_STARTED.with(|cell| cell.get()) {
        return false;
    }
    write_chunked_terminator()
}

// ─────── 单元测试 ───────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_event_with_name() {
        let out = format_event("message", "hello");
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s, "event: message\r\ndata: hello\r\n\r\n");
    }

    #[test]
    fn format_event_no_name() {
        let out = format_event("", "hello");
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s, "data: hello\r\n\r\n");
    }

    #[test]
    fn format_event_multiline() {
        let out = format_event("update", "line1\nline2");
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s, "event: update\r\ndata: line1\r\ndata: line2\r\n\r\n");
    }

    #[test]
    fn format_event_injection_protection() {
        let out = format_event("evil\r\nevent: injected", "data");
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("event: evil"));
        assert!(!s.contains("injected"));
    }

    #[test]
    fn format_event_ends_with_double_crlf() {
        let out = format_event("ping", "");
        let s = String::from_utf8(out).unwrap();
        assert!(s.ends_with("\r\n\r\n"));
    }

    #[test]
    fn default_state_not_started() {
        assert!(!was_sse_started());
    }
}
