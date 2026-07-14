<?php

declare(strict_types=1);

namespace lychee\worker\tests\feature;

use GuzzleHttp\Client;
use PHPUnit\Framework\TestCase;

/**
 * Queue Feature 测试
 *
 * 验证 lychee-worker 与 think-queue 的集成：
 * 一个 HTTP 请求将任务推入队列，队列子进程（或 sync 驱动）立即执行任务。
 */
class QueueTest extends TestCase
{
    use ManagesWorkerServer;

    private ?Client $httpClient = null;
    private static string $markerFile;

    public static function setUpBeforeClass(): void
    {
        if (!extension_loaded('lychee_worker')) {
            self::markTestSkipped('lychee_worker extension is not loaded');
        }

        self::$markerFile = sys_get_temp_dir() . '/lychee_worker_queue_test.log';
        @unlink(self::$markerFile);

        self::startWorker([
            'QUEUE_CONNECTION' => 'sync',
        ]);
    }

    public static function tearDownAfterClass(): void
    {
        self::stopWorker();
    }

    protected function setUp(): void
    {
        $this->httpClient = new Client([
            'base_uri'    => 'http://127.0.0.1:8080',
            'http_errors' => false,
            'timeout'     => 2,
        ]);
        @unlink(self::$markerFile);
    }

    /**
     * End-to-end：HTTP 请求将任务推入队列，队列子进程执行。
     * sync 驱动在 push() 内立即执行任务，验证 think\Queue 的集成路径。
     */
    public function test_queue_job_runs_via_worker(): void
    {
        $response = $this->httpClient->get('/queue');

        $this->assertSame(200, $response->getStatusCode());

        $body = json_decode($response->getBody()->getContents(), true);
        $this->assertIsArray($body);
        $this->assertArrayHasKey('token', $body);
        $this->assertArrayHasKey('path', $body);

        $deadline = microtime(true) + 2;
        while (!is_file($body['path']) && microtime(true) < $deadline) {
            usleep(50_000);
        }

        $this->assertFileExists($body['path'], 'Job marker file was not written; the queue job did not execute.');

        $lines   = array_filter(array_map('trim', file($body['path'])));
        $payload = json_decode(end($lines), true);

        $this->assertSame($body['token'], $payload['data']['token'] ?? null, 'Job payload did not match the token from the push request.');
    }

    /**
     * 单元测试：在 App 实例中直接推送任务，验证 TestJob 类与 sync 队列驱动的工作。
     */
    public function test_sync_queue_push_instantiates_job(): void
    {
        $app = new \think\App(STUB_DIR);
        $app->initialize();

        // 注册 app\ 命名空间自动加载，以便找到 app\job\TestJob
        $loader = new \Composer\Autoload\ClassLoader();
        $loader->addPsr4('app\\', $app->getAppPath());
        $loader->register(true);

        $queue = $app->make('queue');

        $token  = 'unit-' . bin2hex(random_bytes(8));
        $marker = sys_get_temp_dir() . '/lychee_worker_queue_test_unit.log';
        @unlink($marker);

        $queue->push('app\job\TestJob', ['path' => $marker, 'token' => $token]);

        $this->assertFileExists($marker);

        $lines   = array_filter(array_map('trim', file($marker)));
        $payload = json_decode(end($lines), true);

        $this->assertSame($token, $payload['data']['token'] ?? null);

        @unlink($marker);
    }
}
