//! WebSocket 协议处理：升级握手、连接管理、房间广播、消息回调、**心跳检测**。
//!
//! 心跳策略（使用配置文件中的 `ping_interval_sec` / `ping_timeout_sec`）：
//!   1. 服务器定期（`ping_interval_sec`）向客户端发送 Ping 帧
//!   2. 任何收到的帧（Text/Ping/Pong/Binary/Close）都会刷新"最后消息时间戳"
//!   3. 若在 `ping_timeout_sec` 内没有收到任何消息 → 视为连接断开，主动关闭
//!
//! 客户端表现符合 RFC 6455：浏览器的 WebSocket API 会自动回复 Pong。

use ext_php_rs::types::ZendCallable;
use futures_util::{SinkExt, StreamExt};
use sha1::{Digest, Sha1};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::interval;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::runtime::{CONNECTIONS, ROOMS, WS_COUNT};

pub async fn handle_upgrade(
    mut stream: TcpStream,
    pre_read: &[u8],
    ws_open_handler: Option<&'static ZendCallable<'static>>,
    ws_message_handler: Option<&'static ZendCallable<'static>>,
    ws_close_handler: Option<&'static ZendCallable<'static>>,
    ping_interval_sec: u64,
    ping_timeout_sec: u64,
) -> std::io::Result<()> {
    // 从预读取的 HTTP 请求中提取 Sec-WebSocket-Key
    let request_text = String::from_utf8_lossy(pre_read);
    let key = extract_websocket_key(&request_text);

    if let Some(key) = key {
        // 计算 Sec-WebSocket-Accept
        let accept = compute_accept(&key);
        // 写回 101 响应
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {}\r\n\
             \r\n",
            accept
        );
        stream.write_all(response.as_bytes()).await?;
        stream.flush().await?;
    } else {
        // 无效的 WebSocket 升级请求，返回 400
        let response = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(response.as_bytes()).await?;
        stream.flush().await?;
        return Ok(());
    }

    // 心跳参数规范化：最小值保护，避免误填 0 或负数导致死循环
    let (ping_interval, ping_timeout) =
        normalize_heartbeat_config(ping_interval_sec, ping_timeout_sec);

    // WebSocket 帧/消息大小保护（避免超大帧 OOM）
    #[allow(deprecated)]
    let ws_config = WebSocketConfig {
        max_send_queue: None,
        max_message_size: Some(32 * 1024 * 1024),
        max_frame_size: Some(16 * 1024 * 1024),
        max_write_buffer_size: 64 * 1024 * 1024,
        write_buffer_size: 128 * 1024,
        accept_unmasked_frames: false,
    };

    // 握手完成，交给 tungstenite 处理帧
    let ws = WebSocketStream::from_raw_socket(
        stream,
        tokio_tungstenite::tungstenite::protocol::Role::Server,
        Some(ws_config),
    )
    .await;

    let conn_id = alloc_conn_id();

    // 创建消息通道：外部发送者（PHP 侧）-> 写入循环
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    CONNECTIONS.with(|map| {
        map.insert(conn_id.clone(), crate::runtime::SenderCell { sender: tx });
    });
    WS_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    // 通知 PHP 侧：新连接建立
    crate::php_api::call_on_ws_open(ws_open_handler, &conn_id);

    let (mut write, mut read) = ws.split();

    // 主循环：用 tokio::select! 同时等待
    //   ① PHP 侧的推送消息 → write
    //   ② 客户端发来的帧 → read
    //   ③ 定期发送 Ping
    //   ④ 超时检测（超过 ping_timeout_sec 未收到任何消息 → 关闭）
    let mut last_msg_at = Instant::now();
    let mut ping_tick = interval(ping_interval);

    loop {
        tokio::select! {
            // ① PHP 侧推送消息
            maybe_msg = rx.recv() => {
                match maybe_msg {
                    Some(msg) => {
                        if write
                            .send(Message::Text(String::from_utf8_lossy(&msg).into_owned()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    None => break, // 发送端被外部关闭（例如 PHP 主动踢下线）
                }
            }

            // ② 客户端发来的帧
            maybe_frame = read.next() => {
                match maybe_frame {
                    Some(Ok(msg)) => {
                        last_msg_at = Instant::now(); // 刷新"最后消息时间"
                        match msg {
                            Message::Text(text) => {
                                crate::php_api::call_on_ws_message(
                                    ws_message_handler,
                                    &conn_id,
                                    &text,
                                );
                            }
                            Message::Binary(data) => {
                                let payload = String::from_utf8(data).unwrap_or_default();
                                crate::php_api::call_on_ws_message(
                                    ws_message_handler,
                                    &conn_id,
                                    &payload,
                                );
                            }
                            Message::Ping(payload) => {
                                if write.send(Message::Pong(payload)).await.is_err() {
                                    break;
                                }
                            }
                            Message::Pong(_) => {}
                            Message::Close(_) => {
                                let _ = write.close().await;
                                break;
                            }
                            _ => {} // 忽略未识别的帧类型（例如底层的 Frame）
                        }
                    }
                    Some(Err(_)) => break,
                    None => break,
                }
            }

            // ③ 定期发送 Ping
            _ = ping_tick.tick() => {
                if write.send(Message::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }

            // ④ 超时检测：从"最后消息时间"起算，超过 ping_timeout 视为断连
            _ = tokio::time::sleep_until((last_msg_at + ping_timeout).into()) => {
                break;
            }
        }
    }

    // 清理连接
    cleanup_connection(&conn_id);
    crate::php_api::call_on_ws_close(ws_close_handler, &conn_id);
    Ok(())
}

/// 从 HTTP 请求头中提取 Sec-WebSocket-Key（非空才返回 Some）
fn extract_websocket_key(request: &str) -> Option<String> {
    for line in request.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("sec-websocket-key:") {
            let key = line["sec-websocket-key:".len()..].trim();
            if !key.is_empty() {
                return Some(key.to_string());
            }
        }
    }
    None
}

/// 规范化 WebSocket 心跳参数，把"秒"级别的配置转换为
/// `Duration`，并执行最小保护和合理性约束：
///
/// * 最小值保护：`ping_interval_sec` / `ping_timeout_sec` 不低于
///   1 秒（避免误填 0 或负数导致死循环 / 永远不超时）。
/// * 顺序约束：`ping_interval_sec` 必须严格小于 `ping_timeout_sec`；
///   如果开发者把间隔设得大于等于超时，函数会自动把间隔缩小为
///   `ping_timeout - 1` 秒（保证在超时前至少有一次 Ping 机会）。
///
/// 该函数为无副作用的纯函数，便于单元测试。
fn normalize_heartbeat_config(
    ping_interval_sec: u64,
    ping_timeout_sec: u64,
) -> (Duration, Duration) {
    let interval = ping_interval_sec.max(1);
    let timeout = ping_timeout_sec.max(1);

    // 保证 interval < timeout：否则 Ping 发出去还没来得及收到 Pong 就超时。
    // 缩减后再次 max(1) 保护，避免 timeout=1 时得到 interval=0。
    let interval = if interval >= timeout {
        (timeout - 1).max(1)
    } else {
        interval
    };

    (Duration::from_secs(interval), Duration::from_secs(timeout))
}

/// 计算 Sec-WebSocket-Accept 响应值
fn compute_accept(key: &str) -> String {
    let guid = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let concat = format!("{}{}", key, guid);
    let mut hasher = Sha1::new();
    hasher.update(concat.as_bytes());
    let digest = hasher.finalize();
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(digest)
}

fn alloc_conn_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("conn-{}", ns)
}

fn cleanup_connection(conn_id: &str) {
    CONNECTIONS.with(|map| {
        map.remove(conn_id);
    });
    WS_COUNT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    ROOMS.with(|map| {
        for mut entry in map.iter_mut() {
            if let Ok(mut members) = entry.value_mut().members.lock() {
                members.retain(|m| m != conn_id);
            }
        }
    });
}

pub fn send_to(conn_id: &str, data: &[u8]) -> bool {
    CONNECTIONS.with(|map| {
        if let Some(cell) = map.get(conn_id) {
            return cell.sender.send(data.to_vec()).is_ok();
        }
        false
    })
}

pub fn broadcast_all(data: &[u8]) -> bool {
    let payload = data.to_vec();
    CONNECTIONS.with(|map| {
        let mut any = false;
        for cell in map.iter() {
            if cell.value().sender.send(payload.clone()).is_ok() {
                any = true;
            }
        }
        any
    })
}

// 以下函数是公共 API，供 lib.rs 中的 PHP 扩展函数桥接使用。
pub fn broadcast_room(room: &str, data: &[u8]) -> bool {
    let payload = data.to_vec();
    let member_ids: Vec<String> = ROOMS.with(|map| {
        if let Some(cell) = map.get(room) {
            if let Ok(members) = cell.members.lock() {
                return members.clone();
            }
        }
        Vec::new()
    });
    let mut any = false;
    for id in member_ids {
        CONNECTIONS.with(|map| {
            if let Some(cell) = map.get(&id) {
                if cell.sender.send(payload.clone()).is_ok() {
                    any = true;
                }
            }
        });
    }
    any
}

pub fn join_room(conn_id: &str, room: &str) {
    ROOMS.with(|map| {
        if let Some(cell) = map.get(room) {
            if let Ok(mut members) = cell.members.lock() {
                if !members.iter().any(|m| m == conn_id) {
                    members.push(conn_id.to_string());
                }
            }
        } else {
            map.insert(
                room.to_string(),
                crate::runtime::RoomCell {
                    members: std::sync::Mutex::new(vec![conn_id.to_string()]),
                },
            );
        }
    });
}

pub fn leave_room(conn_id: &str, room: &str) {
    ROOMS.with(|map| {
        if let Some(cell) = map.get(room) {
            if let Ok(mut members) = cell.members.lock() {
                members.retain(|m| m != conn_id);
            }
        }
    });
}

pub fn conn_rooms(conn_id: &str) -> Vec<String> {
    ROOMS.with(|map| {
        let mut list = Vec::new();
        for entry in map.iter() {
            if let Ok(members) = entry.value().members.lock() {
                if members.iter().any(|m| m == conn_id) {
                    list.push(entry.key().clone());
                }
            }
        }
        list
    })
}

pub fn room_count(room: &str) -> usize {
    ROOMS.with(|map| {
        if let Some(cell) = map.get(room) {
            if let Ok(members) = cell.members.lock() {
                return members.len();
            }
        }
        0
    })
}

/// 构造 `lychee_worker_ws_emit` 使用的结构化事件：
/// `{"event":<event>,"data":<data>,"ts":<ts>}`
/// 当 `data` 为空字符串时，`data` 字段写 `null`，避免 PHP 侧 `json_decode` 得到空串。
pub fn emit_payload(event: &str, data: &str, ts: u64) -> String {
    let event_json = serde_json::to_string(event).unwrap_or_else(|_| "\"event\"".to_string());
    let data_json = if data.is_empty() {
        "null".to_string()
    } else {
        data.to_string()
    };
    format!(
        "{{\"event\":{},\"data\":{},\"ts\":{}}}",
        event_json, data_json, ts
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_key_from_valid_header() {
        let req = "GET / HTTP/1.1\r\n\
                   Host: localhost\r\n\
                   Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                   Upgrade: websocket\r\n\
                   Connection: Upgrade\r\n\
                   \r\n";
        assert_eq!(
            extract_websocket_key(req),
            Some("dGhlIHNhbXBsZSBub25jZQ==".to_string())
        );
    }

    #[test]
    fn extract_key_line_prefix_is_case_insensitive() {
        let req = "sec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\n";
        assert_eq!(
            extract_websocket_key(req),
            Some("dGhlIHNhbXBsZSBub25jZQ==".to_string())
        );
    }

    #[test]
    fn extract_key_returns_none_when_missing() {
        let req = "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert!(extract_websocket_key(req).is_none());
    }

    #[test]
    fn extract_key_returns_none_when_value_empty() {
        let req = "Sec-WebSocket-Key: \r\n";
        assert!(extract_websocket_key(req).is_none());
    }

    #[test]
    fn compute_accept_matches_rfc6455_example() {
        // RFC 6455 Appendix B 的已知向量
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        assert_eq!(compute_accept(key), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn emit_payload_contains_required_fields() {
        let payload = emit_payload("ping", "{\"count\":1}", 1000);
        assert!(payload.contains("\"event\":\"ping\""));
        assert!(payload.contains("\"data\":{\"count\":1}"));
        assert!(payload.contains("\"ts\":1000"));
    }

    #[test]
    fn emit_payload_empty_data_becomes_null() {
        let payload = emit_payload("ping", "", 1000);
        assert!(payload.contains("\"data\":null"));
        assert!(payload.contains("\"event\":\"ping\""));
    }

    #[test]
    fn emit_payload_escapes_event_string() {
        // 事件名含引号时不能破坏外层 JSON 结构
        let payload = emit_payload("a\"b", "{\"x\":1}", 1);
        assert!(payload.contains("\"event\":\"a\\\"b\""));
    }

    // ─────── 心跳规范化相关测试 ───────

    #[test]
    fn heartbeat_normal_values_pass_through() {
        // 正常配置：25s 间隔 / 60s 超时 → 直接透传
        let (interval, timeout) = normalize_heartbeat_config(25, 60);
        assert_eq!(interval.as_secs(), 25);
        assert_eq!(timeout.as_secs(), 60);
        assert!(interval < timeout);
    }

    #[test]
    fn heartbeat_zero_interval_is_lifted_to_one() {
        // 开发者误填 0 时应有保护，否则 interval=0 会变成无限循环发送 Ping
        let (interval, timeout) = normalize_heartbeat_config(0, 60);
        assert_eq!(interval.as_secs(), 1);
        assert_eq!(timeout.as_secs(), 60);
    }

    #[test]
    fn heartbeat_zero_timeout_is_lifted_to_one() {
        // 误填 timeout=0 → 保护到 1s
        let (interval, timeout) = normalize_heartbeat_config(0, 0);
        assert_eq!(interval.as_secs(), 1);
        assert_eq!(timeout.as_secs(), 1);
    }

    #[test]
    fn heartbeat_interval_greater_than_timeout_is_capped() {
        // 异常配置：间隔 > 超时 → 自动把 interval 缩小到 timeout-1
        let (interval, timeout) = normalize_heartbeat_config(120, 60);
        assert!(
            interval < timeout,
            "interval 必须小于 timeout，否则心跳没有意义"
        );
        assert_eq!(interval.as_secs(), 59);
        assert_eq!(timeout.as_secs(), 60);
    }

    #[test]
    fn heartbeat_interval_equal_to_timeout_is_capped() {
        // 相等时也需要保证 interval < timeout
        let (interval, timeout) = normalize_heartbeat_config(60, 60);
        assert!(interval < timeout);
        assert_eq!(interval.as_secs(), 59);
        assert_eq!(timeout.as_secs(), 60);
    }

    #[test]
    fn heartbeat_production_defaults_match_config_file() {
        // `config/worker.php` 默认值 (25, 60) 必须合法且 interval < timeout
        let (interval, timeout) = normalize_heartbeat_config(25, 60);
        assert!(interval < timeout);
        assert_eq!(interval.as_secs(), 25);
        assert_eq!(timeout.as_secs(), 60);
    }

    #[test]
    fn heartbeat_large_values_are_accepted() {
        // 大配置不溢出、不报错
        let (interval, timeout) = normalize_heartbeat_config(600, 3600);
        assert_eq!(interval.as_secs(), 600);
        assert_eq!(timeout.as_secs(), 3600);
    }
}
