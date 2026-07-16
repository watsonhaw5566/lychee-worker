<?php

declare(strict_types=1);

namespace lychee\worker\command;

use Composer\Autoload\ClassLoader;
use think\App;
use think\console\Command;
use think\console\Input;
use think\console\input\Option;
use think\console\Output;
use think\Cookie;
use think\Response;
use lychee\worker\Websocket;
use RuntimeException;
use Throwable;

/**
 * Lychee Worker — default entry.
 *
 * Starts the HTTP/WebSocket server in production/default mode
 * (no file watcher / hot reload).
 *
 * Usage:
 *   php think worker              # start the server
 *   php think worker --port=9090  # override listening port
 */
class Server extends Command
{
    /**
     * 支持通过 ThinkPHP 容器依赖注入 App 实例。
     * 当 Server 被 WorkerChild 通过 `$app->make(Server::class)` 创建时，
     * 容器会调用此方法注入 `App`，保证 `$this->app` 不为 null。
     * 也支持通过 `Console::addCommand()` 的 `setApp()` 路径设置。
     *
     * @param App $app
     */
    public function __construct(App $app = null)
    {
        if ($app !== null) {
            $this->app = $app;
        }
        parent::__construct();
    }

    /**
     * 获取应用实例（防御式：保证 $this->app 不为 null）。
     * 当通过 `new Server()` 等非常规路径实例化时，回退到全局容器。
     */
    protected function getAppInstance(): App
    {
        if ($this->app === null) {
            if (function_exists('app')) {
                $appInstance = app();
                if ($appInstance instanceof App) {
                    $this->app = $appInstance;
                }
            }
        }
        if ($this->app === null) {
            throw new RuntimeException('Lychee Worker: App container is not available.');
        }

        return $this->app;
    }

    protected function configure(): void
    {
        $this->setName('worker')
            ->setDescription('Lychee Worker — Rust-powered HTTP/WebSocket server (production mode)')
            ->addOption('host', null, Option::VALUE_OPTIONAL, 'Listening address', null)
            ->addOption('port', null, Option::VALUE_OPTIONAL, 'Listening port', null)
            ->addOption('workers', null, Option::VALUE_OPTIONAL, 'Number of worker processes', null);
    }

    protected function execute(Input $input, Output $output): int
    {
        if (!extension_loaded('lychee_worker')) {
            $output->writeln('<error>lychee_worker extension is not loaded</error>');
            $output->writeln('Install it first: <info>bash vendor/watsonhaw/lychee-worker/scripts/install.sh</info>');

            return 1;
        }

        $config = $this->resolveConfigFromInput($input);

        // 生产 / 默认模式：关闭文件 watcher
        $config['watch_dirs']     = '';
        $config['watch_names']    = '';
        $config['watch_excludes'] = '';

        $output->writeln('<info>Lychee Worker starting...</info>');
        $output->writeln("  Listen  : <comment>{$config['host']}:{$config['port']}</comment>");
        $output->writeln("  Workers : <comment>{$config['worker_num']}</comment>");
        $output->writeln("  Queue   : <comment>" . ($config['enable_queue'] ? 'on' : 'off') . '</comment>');
        $output->writeln('  Press Ctrl+C to stop.');
        $output->writeln('');

        return $this->runWorkerLoop($config);
    }

    /**
     * Merge runtime config from config file + CLI arguments (public for WorkerChild).
     */
    public function resolveConfigFromInput(Input $input): array
    {
        $app     = $this->getAppInstance();
        $manager = $app->make('lychee.worker');
        $config  = $manager->getConfig();

        if ($input->hasOption('host') && $input->getOption('host') !== null) {
            $config['host'] = (string) $input->getOption('host');
        }
        if ($input->hasOption('port') && $input->getOption('port') !== null) {
            $config['port'] = (int) $input->getOption('port');
        }
        if ($input->hasOption('workers') && $input->getOption('workers') !== null) {
            $config['worker_num'] = (int) $input->getOption('workers');
        }

        return $config;
    }

    /**
     * Build HTTP/WebSocket handlers and start the Rust runtime (blocks until exit).
     */
    public function runWorkerLoop(array $config): int
    {
        $app = $this->getAppInstance();

        $appLoader = new ClassLoader();
        $appLoader->addPsr4('app\\', $app->getAppPath());
        $appLoader->register(true);
        $publicDir   = public_path();
        $httpHandler = function (string $method, string $path, string $headers, string $body) use ($app, $publicDir): string {
            $headerArray = [];
            $headerLines = preg_split("/\r\n|\n|\r/", trim($headers));
            foreach ($headerLines as $line) {
                $line = trim($line);
                if ($line === '') {
                    continue;
                }
                $colonPos = strpos($line, ':');
                if ($colonPos !== false) {
                    $name                           = trim(substr($line, 0, $colonPos));
                    $value                          = trim(substr($line, $colonPos + 1));
                    $headerArray[strtolower($name)] = $value;
                }
            }

            $_SERVER['REQUEST_METHOD'] = $method;
            $_SERVER['REQUEST_URI']    = $path;
            $_SERVER['SCRIPT_NAME']    = '/index.php';

            foreach ($headerArray as $k => $v) {
                $key = strtoupper(str_replace('-', '_', $k));
                if (!in_array($key, ['CONTENT_TYPE', 'CONTENT_LENGTH'])) {
                    $key = 'HTTP_' . $key;
                }
                $_SERVER[$key] = $v;
            }

            $queryPos = strpos($path, '?');
            if ($queryPos !== false) {
                $queryStr = substr($path, $queryPos + 1);
                parse_str($queryStr, $_GET);
            } else {
                $_GET = [];
            }

            $_POST = [];
            if ($body !== '' && in_array($method, ['POST', 'PUT', 'PATCH'])) {
                $ct = $headerArray['content-type'] ?? '';
                if (str_contains($ct, 'application/json')) {
                    $decoded = json_decode($body, true);
                    if (is_array($decoded)) {
                        $_POST = $decoded;
                    }
                } elseif (str_contains($ct, 'application/x-www-form-urlencoded')) {
                    parse_str($body, $_POST);
                }
            }

            $response = $this->tryStaticFile($publicDir, $path);
            if ($response !== null) {
                return $this->serializeResponse($response);
            }

            try {
                $request  = $app->make('request', [], true);
                $http     = $app->make('think\Http', [], true);
                $response = $http->run($request);

                $cookieObj = $app->make(Cookie::class);
                foreach ($cookieObj->getCookie() as $name => $val) {
                    if (is_string($val[0])) {
                        $response->cookie($name, $val[0]);
                    }
                }
            } catch (Throwable $e) {
                if (function_exists('logger')) {
                    try {
                        logger('lychee_worker')->error($e->getMessage(), [
                            'method' => $method,
                            'path'   => $path,
                            'file'   => $e->getFile(),
                            'line'   => $e->getLine(),
                        ]);
                    } catch (Throwable $logErr) {
                        // 日志系统自身也可能失败，静默忽略
                    }
                }

                $response = Response::create('', 'html', 500);
                $response->header(['Content-Type' => 'text/plain; charset=utf-8']);
            }

            return $this->serializeResponse($response);
        };

        $wsOpenHandler = function (string $connId): void {
            Websocket::joinRoom($connId, 'default');
        };

        $wsMsgHandler = function (string $connId, string $data): void {
            Websocket::broadcastRoom('default', $data);
        };

        $wsCloseHandler = function (string $connId): void {
        };

        // MB → bytes，传给 Rust
        $headerMaxBytes = ((int) $config['header_max_mb']) * 1024 * 1024;
        $bodyMaxBytes   = ((int) $config['body_max_mb'])   * 1024 * 1024;

        \lychee_worker_start(
            $config['host'],
            (int) $config['port'],
            (int) $config['worker_num'],
            $config['enable_queue'],
            $config['watch_dirs'],
            $config['watch_names'],
            $config['watch_excludes'],
            (int) $config['watch_interval_ms'],
            (int) $config['ping_interval_sec'],
            (int) $config['ping_timeout_sec'],
            (int) $config['request_timeout_sec'],
            (int) $config['max_connections'],
            $headerMaxBytes,
            $bodyMaxBytes,
            $httpHandler,
            $wsOpenHandler,
            $wsMsgHandler,
            $wsCloseHandler,
        );

        return 0;
    }

    protected function serializeResponse(Response $response): string
    {
        $body       = $response->getContent();
        $statusCode = $response->getCode() ?: 200;
        $statusText = $this->getHttpStatusText($statusCode);
        $headers    = $response->getHeader();

        $out = "HTTP/1.1 {$statusCode} {$statusText}\r\n";
        $out .= "Content-Length: " . strlen($body) . "\r\n";
        foreach ($headers as $name => $value) {
            if (is_array($value)) {
                foreach ($value as $v) {
                    $out .= "{$name}: {$v}\r\n";
                }
            } elseif ($value !== null && $value !== '') {
                $out .= "{$name}: {$value}\r\n";
            }
        }

        $cookieObj = $response->getCookie();
        if ($cookieObj instanceof Cookie) {
            foreach ($cookieObj->getCookie() as $name => $val) {
                if (is_array($val) && isset($val[0]) && is_string($val[0])) {
                    $out .= "Set-Cookie: {$name}={$val[0]}; path=/\r\n";
                }
            }
        }

        $out .= "\r\n";
        $out .= $body;

        return $out;
    }

    protected function getHttpStatusText(int $code): string
    {
        $texts = [
            100 => 'Continue',
            101 => 'Switching Protocols',
            200 => 'OK',
            201 => 'Created',
            202 => 'Accepted',
            204 => 'No Content',
            301 => 'Moved Permanently',
            302 => 'Found',
            304 => 'Not Modified',
            400 => 'Bad Request',
            401 => 'Unauthorized',
            403 => 'Forbidden',
            404 => 'Not Found',
            405 => 'Method Not Allowed',
            408 => 'Request Timeout',
            500 => 'Internal Server Error',
            502 => 'Bad Gateway',
            503 => 'Service Unavailable',
            504 => 'Gateway Timeout',
        ];

        return $texts[$code] ?? 'Unknown';
    }

    protected function tryStaticFile(string $publicDir, string $path): ?Response
    {
        $cleanPath = str_replace('\\', '/', $path);
        $queryPos  = strpos($cleanPath, '?');
        if ($queryPos !== false) {
            $cleanPath = substr($cleanPath, 0, $queryPos);
        }
        $cleanPath = urldecode($cleanPath);
        if ($cleanPath === '' || $cleanPath === '/') {
            return null;
        }
        $cleanPath = '/' . ltrim($cleanPath, '/');

        $filePath      = $publicDir . str_replace('/', DIRECTORY_SEPARATOR, $cleanPath);
        $realFilePath  = realpath($filePath);
        $realPublicDir = realpath($publicDir);
        if ($realFilePath === false || $realPublicDir === false) {
            return null;
        }
        if (!str_starts_with($realFilePath, rtrim($realPublicDir, DIRECTORY_SEPARATOR) . DIRECTORY_SEPARATOR)) {
            return null;
        }
        if (!is_file($realFilePath)) {
            return null;
        }

        $fileSize = filesize($realFilePath);
        if ($fileSize > 64 * 1024 * 1024) {
            return null;
        }

        $content  = (string) file_get_contents($realFilePath);
        $response = Response::create($content, 'html', 200);
        $mimeType = $this->detectMimeType($realFilePath);
        $response->header([
            'Content-Type'  => $mimeType,
            'Last-Modified' => gmdate('D, d M Y H:i:s', filemtime($realFilePath)) . ' GMT',
            'Cache-Control' => 'public, max-age=31536000',
            'Accept-Ranges' => 'bytes',
        ]);

        return $response;
    }

    protected function detectMimeType(string $filePath): string
    {
        $extension = strtolower(pathinfo($filePath, PATHINFO_EXTENSION));

        $mimeMap = [
            'txt'   => 'text/plain; charset=utf-8',
            'html'  => 'text/html; charset=utf-8',
            'htm'   => 'text/html; charset=utf-8',
            'css'   => 'text/css; charset=utf-8',
            'js'    => 'application/javascript; charset=utf-8',
            'json'  => 'application/json; charset=utf-8',
            'xml'   => 'application/xml; charset=utf-8',
            'svg'   => 'image/svg+xml',
            'png'   => 'image/png',
            'jpg'   => 'image/jpeg',
            'jpeg'  => 'image/jpeg',
            'gif'   => 'image/gif',
            'webp'  => 'image/webp',
            'ico'   => 'image/x-icon',
            'bmp'   => 'image/bmp',
            'pdf'   => 'application/pdf',
            'zip'   => 'application/zip',
            'gz'    => 'application/gzip',
            'tar'   => 'application/x-tar',
            'mp3'   => 'audio/mpeg',
            'mp4'   => 'video/mp4',
            'webm'  => 'video/webm',
            'ogg'   => 'audio/ogg',
            'wav'   => 'audio/wav',
            'woff'  => 'font/woff',
            'woff2' => 'font/woff2',
            'ttf'   => 'font/ttf',
            'otf'   => 'font/otf',
            'eot'   => 'application/vnd.ms-fontobject',
            'csv'   => 'text/csv; charset=utf-8',
            'yml'   => 'application/x-yaml; charset=utf-8',
            'yaml'  => 'application/x-yaml; charset=utf-8',
            'md'    => 'text/markdown; charset=utf-8',
        ];

        if (isset($mimeMap[$extension])) {
            return $mimeMap[$extension];
        }

        if (function_exists('finfo_open')) {
            $finfo = finfo_open(FILEINFO_MIME_TYPE);
            if ($finfo !== false) {
                $mimeType = finfo_file($finfo, $filePath);
                finfo_close($finfo);
                if ($mimeType !== false && $mimeType !== '') {
                    return $mimeType;
                }
            }
        }

        if (function_exists('mime_content_type')) {
            $mimeType = mime_content_type($filePath);
            if ($mimeType !== false && $mimeType !== '') {
                return $mimeType;
            }
        }

        return 'application/octet-stream';
    }
}
