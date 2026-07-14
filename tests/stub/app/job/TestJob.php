<?php

declare(strict_types=1);

namespace app\job;

use think\queue\Job;

class TestJob
{
    /**
     * @param array<string, mixed> $data
     */
    public function fire(Job $job, array $data): void
    {
        $path = $data['path'] ?? (sys_get_temp_dir() . '/think_worker_queue_test.log');

        file_put_contents(
            $path,
            json_encode([
                'time' => microtime(true),
                'data' => $data,
                'pid'  => getmypid(),
            ], JSON_UNESCAPED_SLASHES) . "\n",
            FILE_APPEND | LOCK_EX
        );

        $job->delete();
    }
}
