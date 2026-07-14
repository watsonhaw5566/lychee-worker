<?php

declare(strict_types=1);

namespace lychee\worker;

/**
 * WebSocket 门面类
 *
 * 在 WebSocket 消息/连接回调内，可通过此类静态调用底层 Rust 扩展函数，
 * 完成点对点发送、广播、房间管理等操作。
 *
 * 使用方式：
 *   use lychee\worker\Websocket;
 *
 *   Websocket::send($connId, 'hello');
 *   Websocket::emit($connId, 'chat', json_encode(['msg' => 'hi']));
 *   Websocket::broadcast('system message');
 *   Websocket::joinRoom($connId, 'room1');
 *   Websocket::leaveRoom($connId, 'room1');
 *   Websocket::broadcastRoom('room1', 'hello room');
 *   $rooms   = Websocket::connRooms($connId);  // 连接所在房间列表
 *   $members = Websocket::roomCount('room1');   // 房间成员数
 */
class Websocket
{
    /** 向指定连接发送原始文本 */
    public static function send(string $connId, string $data): bool
    {
        if (!extension_loaded('lychee_worker')) {
            return false;
        }

        return \lychee_worker_ws_send($connId, $data);
    }

    /**
     * 发送结构化事件：包装为 {"event":..., "data":..., "ts":...}
     * @param string $data 预先 JSON 编码后的字符串
     */
    public static function emit(string $connId, string $event, string $data): bool
    {
        if (!extension_loaded('lychee_worker')) {
            return false;
        }

        return \lychee_worker_ws_emit($connId, $event, $data);
    }

    /** 向当前子进程管理的所有连接广播 */
    public static function broadcast(string $data): bool
    {
        if (!extension_loaded('lychee_worker')) {
            return false;
        }

        return \lychee_worker_ws_broadcast($data);
    }

    /** broadcast 的别名（PHP 层代理，不依赖额外 Rust 函数） */
    public static function broadcastAll(string $data): bool
    {
        return self::broadcast($data);
    }

    /** 向指定房间内的连接广播 */
    public static function broadcastRoom(string $room, string $data): bool
    {
        if (!extension_loaded('lychee_worker')) {
            return false;
        }

        return \lychee_worker_ws_broadcast_room($room, $data);
    }

    /** 将连接加入房间 */
    public static function joinRoom(string $connId, string $room): bool
    {
        if (!extension_loaded('lychee_worker')) {
            return false;
        }

        return \lychee_worker_join_room($connId, $room);
    }

    /** 将连接移出房间 */
    public static function leaveRoom(string $connId, string $room): bool
    {
        if (!extension_loaded('lychee_worker')) {
            return false;
        }

        return \lychee_worker_leave_room($connId, $room);
    }

    /** 获取连接所在的所有房间 */
    public static function connRooms(string $connId): array
    {
        if (!extension_loaded('lychee_worker')) {
            return [];
        }

        $rooms = \lychee_worker_conn_rooms($connId);

        return is_array($rooms) ? $rooms : [];
    }

    /** 获取指定房间的成员数 */
    public static function roomCount(string $room): int
    {
        if (!extension_loaded('lychee_worker')) {
            return 0;
        }

        return (int) \lychee_worker_room_count($room);
    }
}
