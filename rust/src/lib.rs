//! lychee-worker —— 基于 Rust (tokio + tokio-tungstenite) 的 PHP 运行时。
//!
//! # 功能概览
//! - Prefork 多进程：父进程 fork N 个 HTTP/WebSocket 子进程，每个子进程跑独立 tokio runtime
//! - HTTP/1.1 处理器：手动解析 method/path/headers/body，调用 PHP 回调返回响应
//! - WebSocket：基于 tokio-tungstenite，支持 open/message/close 回调，进程内房间广播
//! - 队列消费：独立 PHP 子进程 (`php think worker:queue`) 轮询消费队列任务（可选）
//! - 热更新：mtime 监控 + `lychee_worker_trigger_reload()` 手动触发
//! - 运行时统计：`lychee_worker_stats()` 返回 pid/connections/requests/rooms/ws
//!
//! # 注意
//! 每个 HTTP 子进程有独立的 CONNECTIONS / ROOMS / 统计计数器。
//! 当启用多个 worker 时，跨子进程广播需要上层 PHP 业务配合（如 Redis 消息队列）。

use ext_php_rs::builders::ModuleBuilder;
use ext_php_rs::exception::PhpException;
use ext_php_rs::php_function;
use ext_php_rs::php_module;
use ext_php_rs::types::{ZendCallable, Zval};
use ext_php_rs::wrap_function;

mod http;
mod runtime;
mod sse;
mod websocket;

mod php_api;
mod stats;
mod watcher;

/// 解析 PHP 端传入的监控配置字符串（逗号分隔），空字符串返回空 Vec。
fn parse_watch_list(s: &str) -> Vec<String> {
    s.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .collect()
}

#[php_function]
#[allow(clippy::too_many_arguments)]
pub fn lychee_worker_start(
    host: String,
    port: i64,
    worker_num: i64,
    enable_queue: bool,
    watch_dirs: String,
    watch_names: String,
    watch_excludes: String,
    watch_interval_ms: i64,
    ping_interval_sec: i64,
    ping_timeout_sec: i64,
    request_timeout_sec: i64,
    max_connections: i64,
    header_max_bytes: i64,
    body_max_bytes: i64,
    http_handler: Option<&Zval>,
    ws_open_handler: Option<&Zval>,
    ws_message_handler: Option<&Zval>,
    ws_close_handler: Option<&Zval>,
) -> bool {
    let http_callable = http_handler.and_then(|z| ZendCallable::new(z).ok());
    let ws_open_callable = ws_open_handler.and_then(|z| ZendCallable::new(z).ok());
    let ws_message_callable = ws_message_handler.and_then(|z| ZendCallable::new(z).ok());
    let ws_close_callable = ws_close_handler.and_then(|z| ZendCallable::new(z).ok());

    if !(0..=65535).contains(&port) {
        let _ = PhpException::new(
            "invalid port".to_string(),
            0,
            ext_php_rs::zend::ce::exception(),
        )
        .throw();
        return false;
    }
    if worker_num < 1 {
        let _ = PhpException::new(
            "worker_num must be >= 1".to_string(),
            0,
            ext_php_rs::zend::ce::exception(),
        )
        .throw();
        return false;
    }

    let cfg = crate::runtime::WorkerConfig {
        host,
        port: port as u16,
        worker_num: worker_num as usize,
        watch_dirs: parse_watch_list(&watch_dirs),
        watch_names: parse_watch_list(&watch_names),
        watch_excludes: parse_watch_list(&watch_excludes),
        watch_interval_ms: watch_interval_ms as u64,
        ping_interval_sec: ping_interval_sec as u64,
        ping_timeout_sec: ping_timeout_sec as u64,
        enable_queue,
        // 生产环境防护：最小值保护，避免误填 0 或负数
        request_timeout_sec: request_timeout_sec.max(1) as u64,
        max_connections: max_connections.max(1) as usize,
        header_max_bytes: header_max_bytes.max(4096) as usize,
        body_max_bytes: body_max_bytes.max(65536) as usize,
    };

    match crate::runtime::WorkerRuntime::run_blocking(
        cfg,
        http_callable.as_ref(),
        ws_open_callable.as_ref(),
        ws_message_callable.as_ref(),
        ws_close_callable.as_ref(),
    ) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[lychee-worker] runtime error: {e}");
            false
        }
    }
}

/// 向指定 WebSocket 连接发送消息（需在子进程内调用）。
#[php_function]
pub fn lychee_worker_ws_send(conn_id: String, data: String) -> bool {
    crate::websocket::send_to(&conn_id, data.as_bytes())
}

/// 向指定连接发送结构化事件（JSON: {"event":..., "data":..., "ts":...}）。
#[php_function]
pub fn lychee_worker_ws_emit(conn_id: String, event: String, data: String) -> bool {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let payload = crate::websocket::emit_payload(&event, &data, ts);
    crate::websocket::send_to(&conn_id, payload.as_bytes())
}

/// 向所有连接广播消息。
#[php_function]
pub fn lychee_worker_ws_broadcast(data: String) -> bool {
    crate::websocket::broadcast_all(data.as_bytes())
}

/// 向指定房间广播消息。
#[php_function]
pub fn lychee_worker_ws_broadcast_room(room: String, data: String) -> bool {
    crate::websocket::broadcast_room(&room, data.as_bytes())
}

/// 加入房间。
#[php_function]
pub fn lychee_worker_join_room(conn_id: String, room: String) -> bool {
    crate::websocket::join_room(&conn_id, &room);
    true
}

/// 离开房间。
#[php_function]
pub fn lychee_worker_leave_room(conn_id: String, room: String) -> bool {
    crate::websocket::leave_room(&conn_id, &room);
    true
}

/// 查询连接所在房间列表。
#[php_function]
pub fn lychee_worker_conn_rooms(conn_id: String) -> Option<Vec<String>> {
    Some(crate::websocket::conn_rooms(&conn_id))
}

/// 查询指定房间的成员数量。
#[php_function]
pub fn lychee_worker_room_count(room: String) -> i64 {
    crate::websocket::room_count(&room) as i64
}

/// 获取运行时统计信息（键值对）。
#[php_function]
pub fn lychee_worker_stats() -> Option<Vec<(String, i64)>> {
    Some(crate::php_api::stats())
}

/// 手动触发热更新（当前进程退出，由父进程重新 fork）。
#[php_function]
pub fn lychee_worker_trigger_reload() -> bool {
    crate::watcher::trigger_reload()
}

/// 优雅退出。
#[php_function]
pub fn lychee_worker_stop() -> bool {
    crate::runtime::WorkerRuntime::stop();
    true
}

/// 启动 Server-Sent Events 会话。写响应头并切换为 chunked 编码。
/// 必须在同一个 HTTP 请求的 PHP 回调内调用。
#[php_function]
pub fn lychee_worker_sse_start() -> bool {
    crate::sse::start()
}

/// 发送一条 SSE 事件。
///
/// # Arguments
/// * `event` - 事件名（可为空，浏览器默认为 "message"）
/// * `data` - 事件数据，支持多行（\n 分隔）
#[php_function]
pub fn lychee_worker_sse_send(event: String, data: String) -> bool {
    crate::sse::send(event, data)
}

/// 结束 SSE 会话：写入 chunked 终止标记 `0\r\n\r\n` 并刷新缓冲。
#[php_function]
pub fn lychee_worker_sse_end() -> bool {
    crate::sse::end()
}

/// 模块构建入口（ext-php-rs 0.15 要求使用 wrap_function 宏）。
#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    module
        .name("lychee_worker")
        .function(wrap_function!(lychee_worker_start))
        .function(wrap_function!(lychee_worker_ws_send))
        .function(wrap_function!(lychee_worker_ws_emit))
        .function(wrap_function!(lychee_worker_ws_broadcast))
        .function(wrap_function!(lychee_worker_ws_broadcast_room))
        .function(wrap_function!(lychee_worker_join_room))
        .function(wrap_function!(lychee_worker_leave_room))
        .function(wrap_function!(lychee_worker_conn_rooms))
        .function(wrap_function!(lychee_worker_room_count))
        .function(wrap_function!(lychee_worker_stats))
        .function(wrap_function!(lychee_worker_trigger_reload))
        .function(wrap_function!(lychee_worker_stop))
        .function(wrap_function!(lychee_worker_sse_start))
        .function(wrap_function!(lychee_worker_sse_send))
        .function(wrap_function!(lychee_worker_sse_end))
}