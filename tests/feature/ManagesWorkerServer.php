<?php

declare(strict_types=1);

namespace lychee\worker\tests\feature;

use Symfony\Component\Process\Process;
use RuntimeException;
use Throwable;

/**
 * Feature 测试辅助 trait
 *
 * 关键设计：测试服务器用 `LYCHEE_WORKER_CHILD=1` 运行，
 * 这让 Rust 跳过 prefork 循环，直接以**单进程**进入 tokio 事件循环。
 *
 * 注意：`setUpBeforeClass` / `tearDownAfterClass` 是**静态**上下文，
 * PHPUnit 10 的错误/异常处理器在此上下文中会因为"调用栈上没有 TestCase 对象"
 * 而抛出 `NoTestCaseObjectOnCallStackException`。所有可能触发 PHPUnit 错误
 * 处理器的操作（`trigger_error`、即使被 `@` 抑制的 `fsockopen` 等）都必须
 * 用 `self::silence(static fn () => ...)` 包裹，临时恢复默认错误处理器。
 */
trait ManagesWorkerServer
{
    protected static ?Process $process = null;

    /**
     * 在回调执行期间临时禁用 PHPUnit 的错误处理器，
     * 避免在 `setUpBeforeClass` / `tearDownAfterClass` 中触发
     * `NoTestCaseObjectOnCallStackException`。
     *
     * @template T
     * @param callable(): T $fn
     * @return T
     */
    private static function silence(callable $fn): mixed
    {
        $previous = set_error_handler(static fn () => true);

        try {
            return $fn();
        } finally {
            if ($previous !== null) {
                set_error_handler($previous);
            } else {
                restore_error_handler();
            }
        }
    }

    /**
     * 启动 worker 服务器（单进程模式）。
     *
     * @param array<string,string>|null $env 额外的环境变量
     */
    protected static function startWorker(?array $env = null): void
    {
        $env = array_merge([
            'LYCHEE_WORKER_CHILD' => '1',  // ← 关键：跳过 prefork，单进程运行
        ], $env ?? []);

        self::$process = new Process(['php', 'think', 'worker'], STUB_DIR, $env);
        self::$process->start();

        // 等待 HTTP 端口真正可接受连接（Rust tokio 就绪需要一点时间）
        $ready = self::silence(static function () {
            for ($i = 0; $i < 50; $i++) {
                $fp = @fsockopen('127.0.0.1', 8080, $_, $_, 1);
                if ($fp !== false) {
                    fclose($fp);

                    return true;
                }
                usleep(100_000);
            }

            return false;
        });
        if (!$ready) {
            $status = self::$process->isRunning() ? 'running' : 'exited(' . self::$process->getExitCode() . ')';
            $msg    = "worker server did not accept HTTP connections in 5 seconds (status={$status}); "
                . 'stdout=' . self::$process->getOutput()
                . ' stderr=' . self::$process->getErrorOutput();
            self::$process = null;

            throw new RuntimeException($msg);
        }
    }

    protected static function stopWorker(): void
    {
        if (self::$process === null) {
            return;
        }

        // 打印调试信息（便于 CI 排查）
        echo self::$process->getOutput();
        echo self::$process->getErrorOutput();

        // 先尝试优雅停止
        try {
            self::$process->stop(2, SIGTERM);
        } catch (Throwable $e) {
            // 忽略错误，继续兜底
        }

        // 再用 SIGKILL 强制清理
        try {
            if (self::$process->isRunning()) {
                self::$process->stop(2, SIGKILL);
            }
        } catch (Throwable $e) {
            // 忽略
        }

        self::$process = null;

        // 防御性兜底：按端口号杀残留
        self::killByPort(8080);
    }

    /**
     * 用 lsof/fuser 杀掉占用指定端口的所有进程（macOS/Linux）
     */
    protected static function killByPort(int $port): void
    {
        if (PHP_OS_FAMILY === 'Windows') {
            return;
        }

        self::silence(static function () use ($port) {
            @shell_exec("lsof -ti:{$port} 2>/dev/null | xargs -r kill -9 2>/dev/null");
            @shell_exec("fuser -k {$port}/tcp 2>/dev/null");

            // 等待端口真正释放
            $deadline = microtime(true) + 3;
            while (microtime(true) < $deadline) {
                $fp = @fsockopen('127.0.0.1', $port, $_, $_, 1);
                if ($fp === false) {
                    break;
                }
                fclose($fp);
                usleep(100_000);
            }
        });
    }
}
