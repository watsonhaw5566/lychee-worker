<?php

declare(strict_types=1);

namespace lychee\worker\tests\feature;

use GuzzleHttp\Client;
use PHPUnit\Framework\TestCase;

/**
 * SSE (Server-Sent Events) Feature 测试
 *
 * 验证 Rust 扩展中的 `lychee_worker_sse_start / send / end` 系列函数
 * 能够正确地在 HTTP 连接上写入 chunked 编码的 SSE 事件流。
 *
 * 测试策略：
 *   1. 使用 `stream: true` 让 Guzzle 以流式读取 body（避免一次性读完导致等待全部数据）
 *   2. 检查响应头（Content-Type, Cache-Control, 以及 chunked 编码）
 *   3. 解析事件，检查每条事件都有标准格式 `event: ... \ndata: ... \n\n`
 */
class SseTest extends TestCase
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
            'http_errors' => false,
            'timeout'     => 5,
        ]);
    }

    // ────────────────────────────────────────
    // 响应头测试
    // ────────────────────────────────────────

    public function test_sse_response_headers(): void
    {
        $response = $this->httpClient->get('/sse/basic');

        $this->assertSame(200, $response->getStatusCode());
        $this->assertStringContainsString('text/event-stream', $response->getHeaderLine('Content-Type'));
        $this->assertStringContainsString('no-cache', $response->getHeaderLine('Cache-Control'));
        $this->assertStringContainsString('chunked', $response->getHeaderLine('Transfer-Encoding'));
    }

    // ────────────────────────────────────────
    // 基本事件流：3 条事件 + retry 提示
    // ────────────────────────────────────────

    public function test_sse_basic_stream_contains_events(): void
    {
        $response = $this->httpClient->get('/sse/basic');

        $this->assertSame(200, $response->getStatusCode());

        // Guzzle 默认自动解码 Transfer-Encoding: chunked，
        // getContents() 返回原始的 SSE 文本（含 \r\n）
        $body = $response->getBody()->getContents();

        // 首条消息是 retry 提示
        $this->assertStringContainsString('retry: 3000', $body);

        // 三条应用事件
        $this->assertStringContainsString('event: update', $body);
        $this->assertStringContainsString('data: hello-1', $body);
        $this->assertStringContainsString('data: hello-2', $body);
        $this->assertStringContainsString('event: done', $body);
        $this->assertStringContainsString('data: finished', $body);
    }

    public function test_sse_events_are_separated_by_double_newline(): void
    {
        $response = $this->httpClient->get('/sse/basic');

        $this->assertSame(200, $response->getStatusCode());

        $body = $response->getBody()->getContents();

        // 每个事件（包括 retry）都以 \r\n\r\n 或 \n\n 结尾（SSE 规范允许 \n 或 \r\n）
        // 解析为独立事件数组
        $events = $this->parseSseEvents($body);

        // 至少有 4 条：1 条 retry + 3 条业务事件
        $this->assertGreaterThanOrEqual(4, count($events), 'Expected at least 4 SSE events (retry + 3 business)');

        // 第一条非 retry 事件的内容
        $update1 = $this->findEventByName($events, 'update');
        $this->assertNotNull($update1, 'Expected at least one "update" event');
        $this->assertSame('hello-1', $update1['data']);

        $done = $this->findEventByName($events, 'done');
        $this->assertNotNull($done, 'Expected a "done" event');
        $this->assertSame('finished', $done['data']);
    }

    // ────────────────────────────────────────
    // 无 event 名：省略 event 前缀，浏览器默认 "message"
    // ────────────────────────────────────────

    public function test_sse_no_event_name_omits_event_prefix(): void
    {
        $response = $this->httpClient->get('/sse/no-event-name');

        $this->assertSame(200, $response->getStatusCode());

        $body = $response->getBody()->getContents();

        $events = $this->parseSseEvents($body);

        // 找到一条有 data 但无 event 字段的事件
        $found = false;
        foreach ($events as $ev) {
            if ($ev['name'] === '' && str_contains($ev['data'], 'plain-data')) {
                $found = true;
                break;
            }
        }

        $this->assertTrue($found, 'Expected an event without "event:" prefix containing "plain-data"');
    }

    // ────────────────────────────────────────
    // 多行 data：\n → 多个 data: 前缀
    // ────────────────────────────────────────

    public function test_sse_multiline_data_produces_multiple_data_prefixes(): void
    {
        $response = $this->httpClient->get('/sse/multiline-data');

        $this->assertSame(200, $response->getStatusCode());

        $body = $response->getBody()->getContents();

        $events = $this->parseSseEvents($body);

        $msg = $this->findEventByName($events, 'msg');
        $this->assertNotNull($msg, 'Expected a "msg" event');

        // parseSseEvents 会把多行 data: 前缀重新合并为 \n 分隔的字符串
        $this->assertSame("line1\nline2\nline3", $msg['data']);
    }

    // ────────────────────────────────────────
    // 扩展函数可调用性测试
    // ────────────────────────────────────────

    public function test_sse_php_extension_functions_exist(): void
    {
        $this->assertTrue(
            function_exists('lychee_worker_sse_start'),
            'lychee_worker_sse_start() must be declared by the extension'
        );
        $this->assertTrue(
            function_exists('lychee_worker_sse_send'),
            'lychee_worker_sse_send() must be declared by the extension'
        );
        $this->assertTrue(
            function_exists('lychee_worker_sse_end'),
            'lychee_worker_sse_end() must be declared by the extension'
        );
    }

    // ────────────────────────────────────────
    // 辅助：解析 SSE 文本为结构化事件
    // ────────────────────────────────────────

    /**
     * 解析 SSE 文本，返回事件数组。
     *
     * 每条事件结构：
     *   [
     *     'name'  => 'update' | '' (无 event: 前缀时),
     *     'data'  => '合并后的 data 内容（多行用 \n 拼接）',
     *     'retry' => '3000' | '',
     *   ]
     *
     * @return list<array{name:string, data:string, retry:string}>
     */
    private function parseSseEvents(string $body): array
    {
        // 统一行尾：先把 \r\n 转为 \n，再处理单 \r
        $normalized = str_replace(["\r\n", "\r"], "\n", $body);

        // 事件以空行分隔（即 "\n\n"）
        $blocks = preg_split('/\n\s*\n/', $normalized, -1, PREG_SPLIT_NO_EMPTY);

        $events = [];
        foreach ($blocks as $block) {
            $name      = '';
            $dataLines = [];
            $retry     = '';

            foreach (explode("\n", $block) as $line) {
                if ($line === '' || $line[0] === ':') { // 空行或注释
                    continue;
                }

                if (str_starts_with($line, 'event:')) {
                    $name = trim(substr($line, 6));
                } elseif (str_starts_with($line, 'data:')) {
                    $dataLines[] = trim(substr($line, 5));
                } elseif (str_starts_with($line, 'retry:')) {
                    $retry = trim(substr($line, 6));
                }
            }

            $events[] = [
                'name'  => $name,
                'data'  => implode("\n", $dataLines),
                'retry' => $retry,
            ];
        }

        return $events;
    }

    /**
     * 在解析结果中查找第一个指定 event 名的事件。
     *
     * @param list<array{name:string, data:string, retry:string}> $events
     * @return array{name:string, data:string, retry:string}|null
     */
    private function findEventByName(array $events, string $name): ?array
    {
        foreach ($events as $ev) {
            if ($ev['name'] === $name) {
                return $ev;
            }
        }

        return null;
    }
}
