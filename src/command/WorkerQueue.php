<?php

declare(strict_types=1);

namespace lychee\worker\command;

use think\App;
use think\console\Command;
use think\console\Input;
use think\console\Output;
use think\Queue;
use Throwable;
use RuntimeException;

/**
 * 队列子进程入口（由 Rust 父进程 fork 出来，当 enable_queue=true 时）
 *
 * 用法：
 *   php think worker:queue
 */
class WorkerQueue extends Command
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
        $this->setName('worker:queue')
            ->setDescription('Lychee Worker — Queue consumer process');
    }

    protected function execute(Input $input, Output $output): int
    {
        if ($this->app === null) {
            throw new RuntimeException('Lychee Worker (queue): App container is not available.');
        }

        $sleep = 2;

        if (!class_exists(Queue::class)) {
            $output->writeln('<comment>Queue driver not available, exiting.</comment>');

            return 0;
        }

        while (true) {
            try {
                $job = $this->app->make(Queue::class)->pop();
                if ($job === null || $job === false) {
                    sleep($sleep);
                    continue;
                }

                if (is_object($job) && method_exists($job, 'fire')) {
                    $job->fire();
                } elseif (is_callable($job)) {
                    $job();
                }
            } catch (Throwable $e) {
                $output->writeln('<error>Queue error: ' . $e->getMessage() . '</error>');
                sleep($sleep);
            }
        }
    }
}
