<?php

declare(strict_types=1);

namespace lychee\worker\tests\feature;

use GuzzleHttp\Client;
use PHPUnit\Framework\TestCase;
use Ratchet\Client\WebSocket;
use React\EventLoop\Loop;

use function Ratchet\Client\connect;

/**
 * WebSocket Feature 测试
 *
 * 通过 `php think worker start` 启动一个 HTTP/WebSocket 服务器，
 * 验证 WebSocket 连接、消息广播功能。
 */
class WebsocketTest extends TestCase
{
    use ManagesWorkerServer;

    private ?Client $httpClient = null;

    public static function setUpBeforeClass(): void
    {
        if (!extension_loaded('lychee_worker')) {
            self::markTestSkipped('lychee_worker extension is not loaded');
        }

        self::startWorker();
    }

    public static function tearDownAfterClass(): void
    {
        self::stopWorker();
    }

    protected function setUp(): void
    {
        $this->httpClient = new Client([
            'base_uri'    => 'http://127.0.0.1:8080',
            'cookies'     => true,
            'http_errors' => false,
            'timeout'     => 2,
        ]);
    }

    public function test_http(): void
    {
        $response = $this->httpClient->get('/');

        $this->assertSame(200, $response->getStatusCode());
        $this->assertSame('hello world', $response->getBody()->getContents());
    }

    public function test_websocket(): void
    {
        $connected = 0;
        $messages  = [];

        // 客户端 A：连接后只接收
        connect('ws://127.0.0.1:8080/websocket')
            ->then(function (WebSocket $conn) use (&$connected, &$messages) {
                $connected++;
                $conn->on('message', function ($msg) use ($conn, &$messages) {
                    $messages[] = (string) $msg;
                    $conn->close();
                });
            });

        // 客户端 B：连接后发送消息
        connect('ws://127.0.0.1:8080/websocket')
            ->then(function (WebSocket $conn) use (&$connected, &$messages) {
                $connected++;
                $conn->on('message', function ($msg) use ($conn, &$messages) {
                    $messages[] = (string) $msg;
                    $conn->close();
                });

                $conn->send('hello');
            });

        Loop::get()->run();

        $this->assertSame(2, $connected);
        $this->assertSame(['hello', 'hello'], $messages);
    }
}
