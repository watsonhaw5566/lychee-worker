<?php

use think\facade\Queue;
use think\facade\Route;

Route::get('/', function () {
    return 'hello world';
});

Route::put('/', function () {
    return 'put';
});

Route::delete('/', function () {
    return 'delete';
});

Route::get('/queue', function () {
    $path = sys_get_temp_dir() . '/lychee_worker_queue_test.log';
    @unlink($path);

    $token = hash('sha256', random_bytes(16));

    Queue::push('app\job\TestJob', ['path' => $path, 'token' => $token]);

    return json(['token' => $token, 'path' => $path]);
});

// WebSocket 路由：lychee-worker 的 WebSocket 升级通过路由匹配
// 当收到 WebSocket 握手请求时，底层会自动处理握手和消息分发。
// 在这里定义 `/websocket` 路由以便 lychee_worker_start 识别 WebSocket 端点。
Route::get('/websocket', function () {
    // 实际的 WebSocket 握手和消息在 PHP 内核中处理
    return 'websocket';
});

Route::get('test', 'index/test');
Route::post('json', 'index/json');

Route::get('static/:path', function (string $path) {
    $filename = public_path() . $path;
    if (!is_file($filename)) {
        return response('', 404);
    }

    return response(file_get_contents($filename));
})->pattern(['path' => '.*\.\w+$']);

Route::get('/error', function () {
    throw new \RuntimeException('intentional test error');
});
