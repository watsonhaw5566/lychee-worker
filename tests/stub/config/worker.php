<?php

// +----------------------------------------------------------------------
// | Lychee Worker 配置
// +----------------------------------------------------------------------

return [
    'host'              => env('LYCHEE_HOST', '0.0.0.0'),
    'port'              => env('LYCHEE_PORT', 8080),
    'worker_num'        => env('LYCHEE_WORKERS', 2),
    'enable_queue'      => env('LYCHEE_QUEUE', false),
    'watch_dirs'        => 'app,config,route',
    'watch_names'       => '*.php',
    'watch_excludes'    => 'runtime,vendor',
    'watch_interval_ms' => 1000,
    'ping_interval_sec' => 25,
    'ping_timeout_sec'  => 60,
];
