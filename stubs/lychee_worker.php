<?php

/*
 * PHP stubs for the Rust extension `ext-lychee_worker`.
 *
 * Loaded for IDE autocomplete and static analysis (PHPStan / Psalm) so that
 * `lychee_worker_*()` function calls are considered valid even when the
 * extension is not loaded. The guard below ensures we do not conflict with
 * the runtime extension when it is loaded.
 *
 * Function signatures MUST stay in sync with the #[php_function] exports in
 * `rust/src/lib.rs`.
 */

if (!extension_loaded('lychee_worker')) {

    /**
     * Start the lychee-worker runtime. Blocks until the runtime is terminated.
     *
     * The first 14 parameters are runtime configuration; the last 4 are
     * optional PHP callbacks that the Rust side invokes for each event.
     *
     * @param string   $host              Hostname or IP to bind (e.g. "0.0.0.0").
     * @param int      $port              TCP port to bind (1–65535).
     * @param int      $workerNum         Number of HTTP/WebSocket child processes to fork.
     * @param bool     $enableQueue       Whether to also fork a queue consumer process.
     * @param string   $watchDirs         Comma-separated list of directories to scan
     *                                    for mtime changes (e.g. "app,config,route").
     *                                    Empty string = disabled.
     * @param string   $watchNames        Comma-separated list of filename globs
     *                                    (e.g. "*.php"). Empty string = match all files.
     * @param string   $watchExcludes     Comma-separated list of path fragments to
     *                                    exclude from scanning (e.g. "runtime/log,vendor").
     *                                    Empty string = no exclusions.
     * @param int      $watchIntervalMs   Scan interval in milliseconds (default 1000).
     * @param int      $pingIntervalSec   WebSocket PING interval in seconds (default 25).
     * @param int      $pingTimeoutSec    WebSocket ping timeout in seconds (default 60).
     * @param int      $requestTimeoutSec HTTP request timeout in seconds (default 60).
     * @param int      $maxConnections    Maximum concurrent connections (default 1024).
     * @param int      $headerMaxBytes    Maximum request header size in bytes (default 1MB).
     * @param int      $bodyMaxBytes      Maximum request body size in bytes (default 8MB).
     * @param callable|null $httpHandler  HTTP request handler (method, path, headersJson, body) -> full HTTP response text.
     * @param callable|null $wsOpenHandler  WebSocket "open" handler, receives the connection id.
     * @param callable|null $wsMessageHandler WebSocket "message" handler, receives (connId, data).
     * @param callable|null $wsCloseHandler   WebSocket "close" handler, receives the connection id.
     * @return bool True on clean shutdown, false on invalid input.
     */
    function lychee_worker_start(
        string $host,
        int $port,
        int $workerNum,
        bool $enableQueue = false,
        string $watchDirs = '',
        string $watchNames = '',
        string $watchExcludes = '',
        int $watchIntervalMs = 1000,
        int $pingIntervalSec = 25,
        int $pingTimeoutSec = 60,
        int $requestTimeoutSec = 60,
        int $maxConnections = 1024,
        int $headerMaxBytes = 1048576,
        int $bodyMaxBytes = 8388608,
        $httpHandler = null,
        $wsOpenHandler = null,
        $wsMessageHandler = null,
        $wsCloseHandler = null
    ): bool {
        unset($host, $port, $workerNum, $enableQueue, $watchDirs, $watchNames, $watchExcludes, $watchIntervalMs, $pingIntervalSec, $pingTimeoutSec, $requestTimeoutSec, $maxConnections, $headerMaxBytes, $bodyMaxBytes, $httpHandler, $wsOpenHandler, $wsMessageHandler, $wsCloseHandler);

        return false;
    }

    /**
     * Send a raw text/binary message to a specific WebSocket connection.
     *
     * @param string $connId The connection identifier (opaque string).
     * @param string $data   Payload string (may be JSON or binary data).
     * @return bool True on success, false when the connection is unknown.
     */
    function lychee_worker_ws_send(string $connId, string $data): bool
    {
        unset($connId, $data);

        return false;
    }

    /**
     * Emit a structured `{event, data, ts}` WebSocket message.
     *
     * @param string $connId The connection identifier.
     * @param string $event  Event name.
     * @param string $data   JSON payload string (pre-serialized).
     * @return bool True on success, false when the connection is unknown.
     */
    function lychee_worker_ws_emit(string $connId, string $event, string $data): bool
    {
        unset($connId, $event, $data);

        return false;
    }

    /**
     * Broadcast a message to every active WebSocket connection in the current
     * worker process.
     *
     * @param string $data Payload string.
     * @return bool True on success.
     */
    function lychee_worker_ws_broadcast(string $data): bool
    {
        unset($data);

        return false;
    }

    /**
     * Broadcast a message to all WebSocket connections in a specific room.
     *
     * @param string $room Room name.
     * @param string $data Payload string.
     * @return bool True on success.
     */
    function lychee_worker_ws_broadcast_room(string $room, string $data): bool
    {
        unset($room, $data);

        return false;
    }

    /**
     * Add a connection to a room.
     *
     * @param string $connId The connection identifier.
     * @param string $room   Room name.
     * @return bool True on success.
     */
    function lychee_worker_join_room(string $connId, string $room): bool
    {
        unset($connId, $room);

        return false;
    }

    /**
     * Remove a connection from a room.
     *
     * @param string $connId The connection identifier.
     * @param string $room   Room name.
     * @return bool True on success.
     */
    function lychee_worker_leave_room(string $connId, string $room): bool
    {
        unset($connId, $room);

        return false;
    }

    /**
     * Return the list of rooms a specific connection belongs to.
     *
     * @param string $connId The connection identifier.
     * @return array<int, string>
     */
    function lychee_worker_conn_rooms(string $connId): array
    {
        unset($connId);

        return [];
    }

    /**
     * Return the number of connections currently present in a room.
     *
     * @param string $room Room name.
     * @return int
     */
    function lychee_worker_room_count(string $room): int
    {
        unset($room);

        return 0;
    }

    /**
     * Return runtime counters (pid, connections, requests, rooms, websockets…).
     *
     * Counters are scoped to the calling worker process.
     *
     * @return array<string, int>
     */
    function lychee_worker_stats(): array
    {
        return [];
    }

    /**
     * Trigger a graceful hot-reload: the current worker exits and the parent
     * process forks a fresh one.
     *
     * @return bool True on success.
     */
    function lychee_worker_trigger_reload(): bool
    {
        return false;
    }

    /**
     * Request a graceful shutdown of the whole runtime (all workers + parent).
     *
     * @return bool True on success.
     */
    function lychee_worker_stop(): bool
    {
        return false;
    }

    /**
     * Start a Server-Sent Events (SSE) session within the current HTTP request.
     *
     * Writes the SSE response headers and switches to chunked transfer encoding.
     * Must be called from inside the HTTP handler callback.
     *
     * @return bool True on success, false when called outside an HTTP handler.
     */
    function lychee_worker_sse_start(): bool
    {
        return false;
    }

    /**
     * Send a single named SSE event.
     *
     * @param string $event Event name (empty string => browser defaults to "message").
     * @param string $data  Event payload (multi-line strings are split into multiple
     *                      `data:` lines automatically).
     * @return bool True on success, false when SSE session not started.
     */
    function lychee_worker_sse_send(string $event, string $data): bool
    {
        unset($event, $data);

        return false;
    }

    /**
     * End the SSE session: write the chunked terminator `0\r\n\r\n` and flush.
     *
     * @return bool True on success, false when SSE session not started.
     */
    function lychee_worker_sse_end(): bool
    {
        return false;
    }

} // end guard: extension_loaded('lychee_worker')