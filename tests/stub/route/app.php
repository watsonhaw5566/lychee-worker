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

// —— SSE 测试路由 ——
// 基本流：发送 3 条事件后结束
Route::get('/sse/basic', function () {
    if (!function_exists('lychee_worker_sse_start')) {
        return response('SSE extension not loaded', 500);
    }

    \lychee_worker_sse_start();
    \lychee_worker_sse_send('update', 'hello-1');
    \lychee_worker_sse_send('update', 'hello-2');
    \lychee_worker_sse_send('done', 'finished');
    \lychee_worker_sse_end();

    return '';
});

// 无名事件：event 为空时，浏览器默认为 "message" 类型
Route::get('/sse/no-event-name', function () {
    if (!function_exists('lychee_worker_sse_start')) {
        return response('SSE extension not loaded', 500);
    }

    \lychee_worker_sse_start();
    \lychee_worker_sse_send('', 'plain-data');
    \lychee_worker_sse_end();

    return '';
});

// 多行 data：换行符会被拆成多个 data: 前缀
Route::get('/sse/multiline-data', function () {
    if (!function_exists('lychee_worker_sse_start')) {
        return response('SSE extension not loaded', 500);
    }

    \lychee_worker_sse_start();
    \lychee_worker_sse_send('msg', "line1\nline2\nline3");
    \lychee_worker_sse_end();

    return '';
});
