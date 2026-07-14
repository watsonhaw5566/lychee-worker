<?php

declare(strict_types=1);

namespace lychee\worker;

use think\Service as ThinkService;

/**
 * Lychee Worker ThinkPHP 8 服务提供者
 *
 * 由 ThinkPHP 在 composer install 后通过 extra.think.services 自动发现。
 * 负责：
 *   1. 注册命令行指令 `php think worker`
 *   2. 注册 Manager 单例到容器（app('lychee.worker')）
 *   3. 注册 Websocket 门面（app('lychee.websocket')）
 */
class Service extends ThinkService
{
    public function register(): void
    {
        $this->app->bind('lychee.worker', Manager::class);
        $this->app->bind('lychee.websocket', Websocket::class);
    }

    public function boot(): void
    {
        $this->commands([
            command\Server::class,
            command\WorkerDev::class,
            command\WorkerStatus::class,
            command\WorkerChild::class,
            command\WorkerQueue::class,
        ]);
    }
}
