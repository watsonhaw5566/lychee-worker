//! PHP 侧业务回调调用。
//!
//! 回调由 `lychee_worker_start` 直接接收（作为 `ZendCallable` 参数），
//! 在运行时内部直接通过引用调用，不需要长期存储。

use ext_php_rs::types::ZendCallable;

/// 调用 HTTP 处理器，返回响应字符串（可以包含完整 HTTP 响应行+header+body）。
pub fn call_on_http(
    handler: Option<&ZendCallable<'_>>,
    method: &str,
    path: &str,
    headers: &str,
    body: &str,
) -> String {
    if let Some(h) = handler {
        let method_s = method.to_string();
        let path_s = path.to_string();
        let headers_s = headers.to_string();
        let body_s = body.to_string();
        match h.try_call(vec![&method_s, &path_s, &headers_s, &body_s]) {
            Ok(result) => {
                if let Some(s) = result.string() {
                    return s;
                }
            }
            Err(_) => {}
        }
    }
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n<!doctype html><title>lychee-worker</title><h1>lychee-worker {}</h1><p>OK</p>",
        57 + path.len(),
        path
    )
}

/// 调用 WebSocket 连接建立处理器。
pub fn call_on_ws_open(handler: Option<&ZendCallable<'_>>, conn_id: &str) {
    if let Some(h) = handler {
        let _ = h.try_call(vec![&conn_id.to_string()]);
    }
}

/// 调用 WebSocket 消息处理器。
pub fn call_on_ws_message(handler: Option<&ZendCallable<'_>>, conn_id: &str, data: &str) {
    if let Some(h) = handler {
        let _ = h.try_call(vec![&conn_id.to_string(), &data.to_string()]);
    }
}

/// 调用 WebSocket 连接关闭处理器。
pub fn call_on_ws_close(handler: Option<&ZendCallable<'_>>, conn_id: &str) {
    if let Some(h) = handler {
        let _ = h.try_call(vec![&conn_id.to_string()]);
    }
}

/// 返回 PHP 运行时统计信息（键值对）。
pub fn stats() -> Vec<(String, i64)> {
    crate::stats::snapshot().into_iter().collect()
}