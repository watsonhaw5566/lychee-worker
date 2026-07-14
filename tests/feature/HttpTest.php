<?php

declare(strict_types=1);

namespace lychee\worker\tests\feature;

use GuzzleHttp\Client;
use GuzzleHttp\Cookie\CookieJar;
use PHPUnit\Framework\TestCase;

/**
 * HTTP Feature 测试
 *
 * 通过 `php think worker start` 启动一个完整的 HTTP 服务器，
 * 使用真实的 think\App + think\Http 内核处理请求，
 * 验证 GET/POST/PUT/DELETE、Cookie、静态文件等功能正常工作。
 */
class HttpTest extends TestCase
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

    public function test_callback_route(): void
    {
        $response = $this->httpClient->get('/');

        $this->assertSame(200, $response->getStatusCode());
        $this->assertSame('hello world', $response->getBody()->getContents());
    }

    public function test_controller_route(): void
    {
        $jar = new CookieJar();

        $response = $this->httpClient->get('/test', ['cookies' => $jar]);

        $this->assertSame(200, $response->getStatusCode());
        $this->assertSame('test', $response->getBody()->getContents());
        $this->assertSame('think', $jar->getCookieByName('name')->getValue());
    }

    public function test_json_post(): void
    {
        $data = [
            'name' => 'think',
        ];
        $response = $this->httpClient->post('/json', [
            'json' => $data,
        ]);

        $this->assertSame(200, $response->getStatusCode());
        $this->assertSame(json_encode($data), $response->getBody()->getContents());
    }

    public function test_put_and_delete_request(): void
    {
        $response = $this->httpClient->put('/');

        $this->assertSame(200, $response->getStatusCode());
        $this->assertSame('put', $response->getBody()->getContents());

        $response = $this->httpClient->delete('/');

        $this->assertSame(200, $response->getStatusCode());
        $this->assertSame('delete', $response->getBody()->getContents());
    }

    public function test_file_response(): void
    {
        $response = $this->httpClient->get('/static/asset.txt');

        $this->assertSame(200, $response->getStatusCode());
        $this->assertSame(file_get_contents(STUB_DIR . '/public/asset.txt'), $response->getBody()->getContents());
    }

    public function test_static_file_at_public_root(): void
    {
        $response = $this->httpClient->get('/asset.txt');

        $this->assertSame(200, $response->getStatusCode());
        $this->assertSame(file_get_contents(STUB_DIR . '/public/asset.txt'), $response->getBody()->getContents());
    }

    public function test_nonexistent_static_file_returns_404(): void
    {
        $response = $this->httpClient->get('/nonexistent.txt');

        $this->assertSame(404, $response->getStatusCode());
    }

    public function test_static_file_content_type(): void
    {
        $response = $this->httpClient->get('/asset.txt');

        $this->assertSame(200, $response->getStatusCode());
        $this->assertStringContainsString('text/plain', $response->getHeaderLine('Content-Type'));
    }

    public function test_exception_caught_returns_500(): void
    {
        $response = $this->httpClient->get('/error');

        $this->assertSame(500, $response->getStatusCode());
    }

    public function test_response_has_content_length_header(): void
    {
        $response = $this->httpClient->get('/');

        $this->assertSame(200, $response->getStatusCode());
        $this->assertNotEmpty($response->getHeaderLine('Content-Length'));
        $this->assertSame(
            $response->getHeaderLine('Content-Length'),
            (string) strlen($response->getBody()->getContents())
        );
    }

    public function test_path_traversal_prevention(): void
    {
        $response = $this->httpClient->get('/../config/app.php');

        $this->assertSame(404, $response->getStatusCode());

        $response = $this->httpClient->get('/%2e%2e%2fconfig%2fapp.php');

        $this->assertSame(404, $response->getStatusCode());
    }

    public function test_static_file_with_query_string(): void
    {
        $response = $this->httpClient->get('/asset.txt?v=1&t=123');

        $this->assertSame(200, $response->getStatusCode());
        $this->assertSame(file_get_contents(STUB_DIR . '/public/asset.txt'), $response->getBody()->getContents());
    }

    public function test_hot_update(): void
    {
        if (PHP_OS_FAMILY === 'Windows') {
            $this->markTestSkipped('Skip on Windows');
        }

        $response = $this->httpClient->get('/hot');

        $this->assertSame(404, $response->getStatusCode());

        $route = <<<'PHP'
<?php

use think\facade\Route;

Route::get('/hot', function () {
    return 'hot';
});
PHP;

        file_put_contents(STUB_DIR . '/route/hot.php', $route);

        sleep(2);

        try {
            $response = $this->httpClient->get('/hot');

            $this->assertSame(200, $response->getStatusCode());
            $this->assertSame('hot', $response->getBody()->getContents());
        } finally {
            @unlink(STUB_DIR . '/route/hot.php');
        }
    }
}
