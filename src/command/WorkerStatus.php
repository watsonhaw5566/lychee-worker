<?php

declare(strict_types=1);

namespace lychee\worker\command;

use think\App;
use think\console\Command;
use think\console\Input;
use think\console\Output;
use RuntimeException;

/**
 * Show the runtime status of the running Lychee Worker.
 *
 * Must be executed from within a worker process to query internal stats.
 *
 * Usage:
 *   php think worker:status
 */
class WorkerStatus extends Command
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

    /** 将字节数格式化为 "12.34 MB" 之类的人类可读字符串。 */
    private function formatBytes(int $bytes, int $precision = 2): string
    {
        if ($bytes <= 0) {
            return '0 B';
        }
        $units = ['B', 'KB', 'MB', 'GB', 'TB'];
        $i     = (int) floor(log($bytes, 1024));
        if ($i >= count($units)) {
            $i = count($units) - 1;
        }
        $value = $bytes / pow(1024, $i);

        return sprintf("%.{$precision}f %s", $value, $units[$i]);
    }

    protected function configure(): void
    {
        $this->setName('worker:status')
            ->setDescription('Show Lychee Worker runtime status / statistics');
    }

    protected function execute(Input $input, Output $output): int
    {
        if ($this->app === null) {
            throw new RuntimeException('Lychee Worker (status): App container is not available.');
        }

        $manager = $this->app->make('lychee.worker');
        $stats   = $manager->stats();

        if (count($stats) === 0) {
            $output->writeln('<comment>No running Lychee Worker context detected.</comment>');
            $output->writeln('This command reads in-process worker stats — run it from a worker process.');

            return 0;
        }

        $output->writeln('<info>Lychee Worker — Runtime Status</info>');
        $output->writeln('------------------------------------------------');
        foreach ($stats as $key => $value) {
            if ($key === 'memory_rss_kb') {
                continue;
            }
            $output->writeln("  <comment>{$key}</comment> : {$value}");
        }

        if (isset($stats['memory_rss_kb']) && (int) $stats['memory_rss_kb'] > 0) {
            $readable = $this->formatBytes((int) $stats['memory_rss_kb'] * 1024);
            $output->writeln("  <comment>memory</comment> : {$readable}");
        }

        $output->writeln('------------------------------------------------');

        return 0;
    }
}
