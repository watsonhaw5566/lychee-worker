//! WebSocket 协议处理：升级握手、连接管理、房间广播、消息回调。

use ext_php_rs::types::ZendCallable;
use futures_util::{SinkExt, StreamExt};
use sha1::{Digest, Sha1};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::runtime::{CONNECTIONS, ROOMS, WS_COUNT};

pub async fn handle_upgrade(
    mut stream: TcpStream,
    pre_read: &[u8],
    ws_open_handler: Option<&'static ZendCallable<'static>>,
    ws_message_handler: Option<&'static ZendCallable<'static>>,
    ws_close_handler: Option<&'static ZendCallable<'static>>,
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

    // 握手完成，交给 tungstenite 处理帧
    let ws = WebSocketStream::from_raw_socket(
        stream,
        tokio_tungstenite::tungstenite::protocol::Role::Server,
        None,
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

    let conn_id_for_async = conn_id.clone();
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            // 写入循环：接收来自 PHP 侧的推送消息
            let writer = tokio::task::spawn_local(async move {
                while let Some(msg) = rx.recv().await {
                    if write
                        .send(Message::Text(String::from_utf8_lossy(&msg).into_owned()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                let _ = write.close().await;
            });

            // 读取循环：接收客户端消息 -> 转交 PHP 回调
            let reader_conn_id = conn_id_for_async.clone();
            let reader = tokio::task::spawn_local(async move {
                while let Some(Ok(msg)) = read.next().await {
                    if let Message::Text(text) = msg {
                        crate::php_api::call_on_ws_message(
                            ws_message_handler,
                            &reader_conn_id,
                            &text,
                        );
                    }
                }
            });

            let _ = tokio::join!(reader, writer);
        })
        .await;

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
}
