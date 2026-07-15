<?php

declare(strict_types=1);

namespace lychee\worker\command;

use think\console\Input;
use think\console\Output;

/**
 * Lychee Worker — development mode with hot reload (file watcher).
 *
 * Usage:
 *   php think worker:dev              # start with hot reload
 *   php think worker:dev --port=9090  # override port
 */
class WorkerDev extends Server
{
    protected function configure(): void
    {
        parent::configure();

        $this->setName('worker:dev')
            ->setDescription('Lychee Worker — development mode with hot reload (file watcher)');
    }

    protected function execute(Input $input, Output $output): int
    {
        if (!extension_loaded('lychee_worker')) {
            $output->writeln('<error>lychee_worker extension is not loaded</error>');
            $output->writeln('Install it first: <info>bash vendor/watsonhaw/lychee-worker/scripts/install.sh</info>');

            return 1;
        }

        $config = $this->resolveConfigFromInput($input);

        // dev 模式：启用文件 watcher 与热更新（配置来源于 config/worker.php）
        $output->writeln('<info>Lychee Worker starting in dev mode...</info>');
        $output->writeln("  Listen  : <comment>{$config['host']}:{$config['port']}</comment>");
        $output->writeln("  Workers : <comment>{$config['worker_num']}</comment>");
        $output->writeln("  Queue   : <comment>" . ($config['enable_queue'] ? 'on' : 'off') . '</comment>');
        $output->writeln("  Watch   : <comment>{$config['watch_dirs']}</comment> (<comment>{$config['watch_names']}</comment>)");
        $output->writeln('');

        return $this->runWorkerLoop($config);
    }
}
