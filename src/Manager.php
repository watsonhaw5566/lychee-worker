<?php

declare(strict_types=1);

namespace lychee\worker;

use think\App;
use think\facade\Config;
use RuntimeException;

/**
 * Lychee Worker 管理器
 *
 * 封装底层 Rust PHP 扩展（lychee_worker_* 系列函数），
 * 对外提供符合 ThinkPHP 风格的面向对象 API。
 *
 * 使用方式：
 *   $manager = app('lychee.worker');
 *   $manager->start(fn($m, $p, $h, $b) => 'HTTP/1.1 200...');
 *   $manager->stats();  // ['connections' => N, 'rooms' => N, 'requests' => N, 'ws' => N]
 */
class Manager
{
    protected App $app;

    /** @var array 运行时配置 */
    protected array $config;

    public function __construct(App $app)
    {
        $this->app    = $app;
        $this->config = Config::get('worker', []) + $this->defaultConfig();
    }

    /**
     * 启动 lychee-worker 运行时（阻塞方法）
     *
     * 在 ThinkPHP 项目中通常由 `php think worker` 调用。
     *
     * @param callable|null $httpHandler    HTTP 请求处理器 (method, path, headersJson, body) => fullHttpResponse
     * @param callable|null $wsOpenHandler WebSocket 连接建立 (connId) => void
     * @param callable|null $wsMsgHandler  WebSocket 消息到达 (connId, data) => void
     * @param callable|null $wsCloseHandler WebSocket 连接关闭 (connId) => void
     * @return bool
     * @throws RuntimeException 如果扩展未加载
     */
    public function start(
        ?callable $httpHandler = null,
        ?callable $wsOpenHandler = null,
        ?callable $wsMsgHandler = null,
        ?callable $wsCloseHandler = null,
    ): bool {
        if (!extension_loaded('lychee_worker')) {
            throw new RuntimeException(
                'lychee_worker extension not loaded — please run: ' .
                'bash vendor/watsonhaw/lychee-worker/scripts/install.sh'
            );
        }

        return \lychee_worker_start(
            (string) $this->config['host'],
            (int)    $this->config['port'],
            (int)    $this->config['worker_num'],
            (bool)   $this->config['enable_queue'],
            (string) $this->config['watch_dirs'],
            (string) $this->config['watch_names'],
            (string) $this->config['watch_excludes'],
            (int)    $this->config['watch_interval_ms'],
            (int)    $this->config['ping_interval_sec'],
            (int)    $this->config['ping_timeout_sec'],
            $httpHandler,
            $wsOpenHandler,
            $wsMsgHandler,
            $wsCloseHandler,
        );
    }

    /** 获取运行时统计信息（键值对） */
    public function stats(): array
    {
        if (!extension_loaded('lychee_worker')) {
            return [];
        }

        $stats = \lychee_worker_stats();

        return is_array($stats) ? $stats : [];
    }

    /** 获取配置副本 */
    public function getConfig(): array
    {
        return $this->config;
    }

    /** 默认配置（当 config/worker.php 不存在时使用） */
    protected function defaultConfig(): array
    {
        return [
            'host'              => '0.0.0.0',
            'port'              => 8080,
            'worker_num'        => 2,
            'enable_queue'      => false,
            'watch_dirs'        => 'app,config,route',
            'watch_names'       => '*.php',
            'watch_excludes'    => 'runtime,vendor',
            'watch_interval_ms' => 1000,
            'ping_interval_sec' => 25,
            'ping_timeout_sec'  => 60,
        ];
    }
}