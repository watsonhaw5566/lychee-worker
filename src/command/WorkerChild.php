<?php

declare(strict_types=1);

namespace lychee\worker\command;

use think\App;
use think\console\Command;
use think\console\Input;
use think\console\input\Option;
use think\console\Output;
use RuntimeException;

/**
 * HTTP Worker 子进程入口（由 Rust 父进程 fork 出来，设置 LYCHEE_WORKER_CHILD=1）
 *
 * 用法（不手动调用）：
 *   php think worker:child --host=127.0.0.1 --port=8080 --worker-num=4
 */
class WorkerChild extends Command
{
    /**
     * 支持容器注入 App，保证 $this->app 不为 null。
     */
    public function __construct(App $app = null)
    {
        if ($app !== null) {
            $this->app = $app;
        }
        parent::__construct();
    }

    protected function configure(): void
    {
        $this->setName('worker:child')
            ->setDescription('Lychee Worker — HTTP/WebSocket child process')
            ->addOption('host', null, Option::VALUE_OPTIONAL, 'Listening address', null)
            ->addOption('port', null, Option::VALUE_OPTIONAL, 'Listening port', null)
            ->addOption('worker-num', null, Option::VALUE_OPTIONAL, 'Worker count', null);
    }

    protected function execute(Input $input, Output $output): int
    {
        if (!extension_loaded('lychee_worker')) {
            $output->writeln('<error>lychee_worker extension is not loaded</error>');

            return 1;
        }

        if ($this->app === null) {
            throw new RuntimeException('Lychee Worker (child): App container is not available.');
        }

        $server = $this->app->make(Server::class);
        $config = $server->resolveConfigFromInput($input);
        $server->runWorkerLoop($config);

        return 0;
    }
}
