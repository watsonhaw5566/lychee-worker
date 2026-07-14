<?php

declare(strict_types=1);

/**
 * Lychee Worker 默认配置
 *
 * 在 ThinkPHP 8 项目中，`env()` 函数是框架的全局辅助函数。
 * 在非 ThinkPHP 环境（例如单元测试）下，退回到原生 `getenv()`。
 *
 */

return [
    // HTTP/WebSocket 监听地址
    'host'              => env('LYCHEE_HOST', '0.0.0.0'),
    // 监听端口
    'port'              => env('LYCHEE_PORT', 8080),
    // 子进程数量（prefork 模式）
    'worker_num'        => env('LYCHEE_WORKERS', 2),
    // 是否额外 fork 一个队列消费子进程
    'enable_queue'      => env('LYCHEE_QUEUE', false),
    // 热更新监控目录（逗号分隔）
    'watch_dirs'        => env('LYCHEE_WATCH_DIRS', 'app,config,route'),
    // 被监控文件名模式（逗号分隔 glob）
    'watch_names'       => env('LYCHEE_WATCH_NAMES', '*.php'),
    // 排除规则（逗号分隔路径片段）
    'watch_excludes'    => env('LYCHEE_WATCH_EXCLUDES', 'runtime,vendor'),
    // 轮询间隔（毫秒）
    'watch_interval_ms' => env('LYCHEE_WATCH_INTERVAL', 1000),
    // WebSocket 心跳发包间隔（秒）
    'ping_interval_sec' => env('LYCHEE_PING_INTERVAL', 25),
    // WebSocket 心跳超时（秒）
    'ping_timeout_sec'  => env('LYCHEE_PING_TIMEOUT', 60),

];
