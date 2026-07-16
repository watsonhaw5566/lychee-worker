# lychee-worker

基于 Rust (tokio + tokio-tungstenite) 重写的轻量级 PHP 运行时，同时提供 **ThinkPHP 8 插件** 封装，为 ThinkPHP 项目提供高性能的 HTTP/WebSocket 服务。

本项目包含两部分：

1. **Rust PHP 扩展**（`lychee_worker.so` / `lychee_worker.dylib`） — 向 PHP 导出 `lychee_worker_start` 等一整套原生函数。真正的 HTTP / WebSocket / 房间 / 广播 / 热更新 / 信号处理都在扩展里以原生 Rust 跑。
2. **ThinkPHP 8 插件**（`src/` 目录） — 符合 PSR-4，命名空间 `lychee\worker\`，通过 `composer.json` 的 `extra.think.services` 自动注册到 ThinkPHP 容器，提供 `php think worker` 命令行入口、`Manager` 管理器和 `Websocket` 门面类。

## 特性

- **Prefork 多进程**：父进程 fork N 个子进程，每个子进程跑独立的 tokio 事件循环，避免 PHP ZTS 问题；主进程 + 子进程总数显示在控制台
- **原生 HTTP/1.1**：纯 Rust 手动解析（method / path / headers / body），零依赖 FPM/nginx
- **原生 WebSocket**：tokio-tungstenite，支持连接建立/消息/关闭回调，进程内房间广播
- **单点发送与广播**：`lychee_worker_ws_send` / `lychee_worker_ws_emit` / `lychee_worker_ws_broadcast` / `lychee_worker_ws_broadcast_room`
- **房间管理**：`lychee_worker_join_room` / `lychee_worker_leave_room` / `lychee_worker_conn_rooms` / `lychee_worker_room_count`（进程内，暂无跨进程 IPC）
- **队列消费**（可选）：独立 PHP 子进程 (`php think worker:queue`) 轮询消费已注册队列任务
- **热更新**：mtime 轮询，检测到 app/config/route 目录文件变化后自动重启所有子进程；也可通过 `lychee_worker_trigger_reload()` 手动触发
- **优雅关闭**：捕获 SIGINT/SIGTERM，关闭所有子进程后退出
- **运行时统计与内存**：`lychee_worker_stats()` 返回 connections / requests / rooms / ws / memory（当前进程物理内存 RSS，单位 MB）。
- **控制台输出**：启动表格显示协议、监听地址、进程数、状态（类似 Workerman 风格）

## 运行环境要求

| 组件 | 最低版本 | 说明 |
|---|---|---|
| PHP | 8.3 | 需要 `php-config`、`phpize` 等开发工具（通常由 `php-dev` / `php-devel` 包提供）。仅支持 PHP 8.3；ext-php-rs 在构建时通过 `php-config` 探测版本 |
| Rust | 1.70+ | 推荐通过 rustup 安装 |
| 操作系统 | Linux / macOS | 当前仅在 Linux/macOS 验证构建与运行 |

## ThinkPHP 8 命令一览

安装插件后，`php think` 会多出以下命令：

| 命令 | 说明 |
|---|---|
| `php think worker` | **生产模式**：启动 HTTP/WebSocket 服务器，文件 watcher 关闭（无热更新） |
| `php think worker:dev` | **开发模式**：同 `worker`，但启用文件监控与热更新（`app/config/route/*.php` 变化时自动重启子进程） |
| `php think worker:queue` | **队列消费进程**（由 `enable_queue=true` 时 Rust 侧自动 fork，一般不需手动调用） |
| `php think worker:child` | **HTTP 子进程入口**（由 Rust 侧 fork，一般不需手动调用） |
| `php think worker:status` | 查询运行时统计：connections / ws / requests / rooms / **memory**（MB）。直接执行，无需在 worker 进程内调用 |

`worker` 与 `worker:dev` 均支持 `--host`、`--port`、`--workers` 参数覆盖配置文件：

```bash
php think worker --host=127.0.0.1 --port=9090 --workers=4
```

## 安装

本项目同时提供 `pie.json`（声明 `type: php-ext`），可被 [PIE](https://php.github.io/pie/)（PHP Installer for Extensions）识别为 PHP 扩展并构建；`composer.json` 的 `type` 为标准 `library`，确保 ThinkPHP 项目的 `extra.think` 自动发现与 Composer autoload 正常工作。

### 前置条件

- PHP 8.3（含 `php-config`）
- 若使用**免构建**安装（推荐）：安装 [PIE](https://php.github.io/pie/)（PHP Installer for Extensions）即可，**无需 Rust 工具链**
- 若要从源码构建：Rust 工具链（`rustup install stable && rustup default stable`）

### 一键安装（PIE · 免构建，推荐）

`pie.json` 已声明 `download-url-method: [pre-packaged-binary, composer-default]`，PIE 会优先从 GitHub Release 下载当前平台的预编译 `.so`/`.dylib`，**不触发 cargo build**，也就不依赖 Rust 工具链：

```bash
pie install watsonhaw/lychee-worker
```

若当前平台没有预编译产物，PIE 会自动回退到 `composer-default`（下载源码 → configure → make → make install），此时才需要 Rust 工具链。

### 一键安装（PIE · 源码构建）

当 PIE 回退到源码构建时，它会自动：
1. 下载源码
2. 执行 `./configure`（检测工具链并生成 Makefile）
3. 执行 `make`（底层为 `cargo build --release`）
4. 执行 `make install`（底层为 `bash scripts/install.sh`，拷贝 `.so` 到扩展目录并写入 `php.ini`）

### 在 ThinkPHP 8 项目中安装（Composer 路径）

```bash
# 进入你的 ThinkPHP 8 项目根目录
cd /path/to/your-thinkphp-project

# 1. 允许 dev 稳定性并拉取包
composer config minimum-stability dev
composer config prefer-stable true
composer require watsonhaw/lychee-worker

# 2. 编译并安装 Rust 扩展
bash vendor/watsonhaw/lychee-worker/scripts/install.sh

# 3. 验证扩展是否已加载
php -m | grep lychee_worker

# 4. 启动服务
php think worker
```

### 分步编译

如果不想用 PIE，也可以手动分步完成构建。以 ThinkPHP 8 项目为例，`composer require` 后进入包目录执行：

```bash
cd vendor/watsonhaw/lychee-worker

# 步骤 1：编译 Rust 扩展（release 模式，产物在 target/release/）
cargo build --release

# 步骤 2：将 .so / .dylib 拷贝到 PHP 扩展目录，并写入 php.ini
bash scripts/install.sh

# 步骤 3：验证扩展是否已加载
php -m | grep lychee_worker
```

`scripts/install.sh` 会自动：
1. 检测当前操作系统（Linux 输出 `.so`，macOS 输出 `.dylib`）
2. 若 `target/release/liblychee_worker.*` 不存在，自动执行 `cargo build --release`
3. 把编译产物复制到 `php-config --extension-dir` 指向的目录
4. 在 macOS 上执行 `codesign --force --deep -s -` 以通过 SIP
5. 自动在已加载的 `php.ini`（或 `conf.d/99-lychee_worker.ini`）追加 `extension=/absolute/path/to/lychee_worker.so`
6. 用 `php -m` 验证扩展是否被成功加载

`scripts/install.sh` 支持以下参数：

```bash
bash scripts/install.sh                                # 编译 + 复制 + 写入 php.ini（默认）
bash scripts/install.sh --no-ini                       # 只复制扩展文件，不修改 php.ini
bash scripts/install.sh --ini=/path/to/custom/php.ini  # 指定自定义 php.ini 路径
bash scripts/install.sh --from-github-release=0.1.0    # 从 GitHub Release 下载预编译二进制，
                                                       # 不执行 cargo build（无需 Rust 工具链）
bash scripts/install.sh --from-github-release=latest   # 使用最新 release 的预编译产物
```

> 使用 `--from-github-release` 时，脚本会根据当前 OS / 架构 / libc / PHP 版本生成 PIE
> 预编译包名 `php_lychee_worker-<tag>_php8.3-<arch>-<OS>-<libc>-release-nts.zip`，并从
> GitHub Release 的浏览器下载链接拉取该 ZIP；对历史版本会回退到原始
> `liblychee_worker-<OS>.so` 命名。下载后如果是 ZIP 会自动解包，然后再执行拷贝和
> php.ini 注入。

### 直接下载预编译产物（无 PIE / 无 install.sh）

Release 页会同时挂出符合 PIE 命名规范的 ZIP 包，以及 `liblychee_worker-<OS>-<arch>.so`
原始文件。只需：

```bash
wget https://github.com/watsonhaw5566/lychee-worker/releases/latest/download/liblychee_worker-$(uname)-x86_64.so
cp liblychee_worker-*.so "$(php-config --extension-dir)/lychee_worker.so"
echo 'extension=lychee_worker.so' >> "$(php --ini | awk -F': ' '/Loaded Configuration File/ {print $2; exit}')"
php -m | grep lychee_worker
```

### 在包源码目录中构建（开发/测试场景）

如果是直接克隆 `lychee-worker` 源码进行开发，无需进入 `vendor/`，在项目根目录即可：

```bash
cd /path/to/lychee-worker

# 方法 A：Composer 脚本别名
composer run-script install-ext

# 方法 B：直接执行脚本
bash scripts/install.sh

# 方法 C：完全手动
cargo build --release
cp target/release/liblychee_worker.dylib "$(php-config --extension-dir)/lychee_worker.dylib"
echo "extension=$(php-config --extension-dir)/lychee_worker.dylib" >> "$(php --ini | awk -F': ' '/Loaded Configuration File/ {print $2; exit}')"
php -m | grep lychee_worker
```

### 验证

```bash
php -m | grep lychee_worker
```

若上面的命令没有任何输出，说明扩展未被加载。请检查：
1. `php-config --extension-dir` 目录下是否存在 `lychee_worker.so`（Linux）或 `lychee_worker.dylib`（macOS）
2. 当前执行的 `php` 所使用的 `php.ini`（通过 `php --ini` 查看）是否包含 `extension=.../lychee_worker.*` 这一行
3. macOS 下如果提示签名相关错误，可重新执行 `bash scripts/install.sh`，脚本会自动 `codesign`

> **说明**：包内的 ThinkPHP 插件类（`src/` 下的 PSR-4 代码）由 Composer 正常 autoload，ThinkPHP 项目安装扩展后即可使用 `php think worker` 命令启动服务。

## 升级与卸载

```bash
# 升级
composer update watsonhaw/lychee-worker
bash vendor/watsonhaw/lychee-worker/scripts/install.sh

# 卸载
php -r "echo ini_get('extension_dir') . PHP_EOL;"
# 删除上面目录里的 lychee_worker.so / lychee_worker.dylib
# 并从 php.ini 移除 extension=lychee_worker 这行
composer remove watsonhaw/lychee-worker
```

## 最小使用示例

把下面内容保存为 `server.php`，然后 `php server.php`：

```php
<?php

if (!extension_loaded('lychee_worker')) {
    fwrite(STDERR, "error: lychee_worker extension is not loaded\n");
    exit(1);
}

lychee_worker_start(
    '0.0.0.0',          // host
    8080,               // port
    2,                  // worker_num（HTTP 子进程数）
    false,              // enable_queue — 是否额外 fork queue 消费子进程
    'app,config',       // watch_dirs — 逗号分隔的热更新监控目录
    '*.php',            // watch_names — 被监控文件名 glob
    '',                 // watch_excludes — 排除规则 glob，空字符串表示不排除
    1000,               // watch_interval_ms — 热更新轮询间隔
    25,                 // ping_interval_sec — WebSocket 心跳发包间隔
    60,                 // ping_timeout_sec — WebSocket 心跳超时
    // --- 以下为 PHP 侧回调，传 null 表示使用扩展默认行为 ---
    function (string $method, string $path, string $headers, string $body): string {
        return "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n"
             . "<h1>lychee-worker</h1><p>$method $path</p>";
    },
    function (string $connId): void {
        // WebSocket 连接建立
    },
    function (string $connId, string $data): void {
        // 收到消息：加入房间 chat，并广播结构化事件
        lychee_worker_join_room($connId, 'chat');
        lychee_worker_ws_broadcast_room(
            'chat',
            json_encode(['event' => 'msg', 'data' => $data])
        );
    },
    function (string $connId): void {
        // 连接关闭
    }
);
```

启动：

```bash
php server.php
```

然后浏览器打开 `http://localhost:8080/` 验证 HTTP，用 WebSocket 客户端连接 `ws://localhost:8080/` 验证消息。

### 关于 `lychee_worker_ws_emit`

`lychee_worker_ws_emit(conn_id, event, data)` 会自动把数据包成：

```json
{"event":"<event>","data":<data>,"ts":<unix秒>}
```

`data` 必须是**合法的 JSON 值**（字符串/对象/数组/数字，都要自己先 JSON 编码一次）。如果 `data` 为空字符串，扩展会把它置为 `null`。

## PHP 扩展函数（ext-lychee_worker）

| 函数 | 说明 |
|---|---|
| `lychee_worker_start(host, port, worker_num, enable_queue, watch_dirs, watch_names, watch_excludes, watch_interval_ms, ping_interval_sec, ping_timeout_sec, on_http, on_ws_open, on_ws_message, on_ws_close)` | 启动运行时（阻塞）；enable_queue 为 true 时额外 fork 1 个 queue 消费子进程 |
| `lychee_worker_stop()` | 触发优雅关闭 |
| `lychee_worker_trigger_reload()` | 手动触发热更新（当前子进程退出，由父进程重新 fork） |
| `lychee_worker_stats()` | 返回运行时统计（关联数组 `{connections, ws, requests, rooms, memory_rss_kb}`，值均为整数；`memory_rss_kb` 为当前进程物理内存 RSS，单位 KB） |
| `lychee_worker_ws_send(conn_id, data)` | 向指定连接发送文本消息 |
| `lychee_worker_ws_emit(conn_id, event, data)` | 发送结构化事件：`{"event":..., "data":..., "ts":...}` |
| `lychee_worker_ws_broadcast(data)` | 向当前子进程管理的所有连接广播文本 |
| `lychee_worker_ws_broadcast_all(data)` | `lychee_worker_ws_broadcast` 的别名 |
| `lychee_worker_ws_broadcast_room(room, data)` | 向指定房间内的连接广播 |
| `lychee_worker_join_room(conn_id, room)` | 将连接加入房间 |
| `lychee_worker_leave_room(conn_id, room)` | 将连接移出房间 |
| `lychee_worker_conn_rooms(conn_id)` | 返回连接所在房间列表（`array<string>`） |
| `lychee_worker_room_count(room)` | 返回指定房间的成员数（`int`） |

> 注：所有扩展函数同时也可通过 `lychee\worker\Manager` 和 `lychee\worker\Websocket` 的面向对象 API 调用。

### 关于"房间/广播是进程内的"

每个 HTTP 子进程有独立的连接表与房间表。启用多个 worker 时，`lychee_worker_ws_broadcast_room` 只在**同一个子进程**里广播。如果你的业务需要跨子进程广播，建议在 PHP 侧把消息推到 Redis/消息队列，再由各个子进程消费后调用 `lychee_worker_ws_broadcast_*`。

## 常见问题

**Q1：`php -m` 没看到 `lychee_worker`？**

检查三件事：
1. `target/release/` 下确实有 `liblychee_worker.so` 或 `liblychee_worker.dylib`
2. 文件已拷贝到 `php-config --extension-dir` 指向的目录
3. 正在编辑的 `php.ini` 是 `php --ini` 里显示的那个文件（CLI 与 FPM 经常不在同一个 php.ini）

**Q2：构建时提示找不到 `php-config`？**

说明只装了 PHP 运行时，没装开发包。在 Debian/Ubuntu：`sudo apt install php8.3-dev`；在 macOS（Homebrew）：`brew install php` 自带；在 Fedora/RHEL：`sudo dnf install php-devel`。

**Q3：只想看函数签名做 IDE 提示，不想构建？**

包内提供了 `stubs/lychee_worker.php`，纯 PHP 的函数签名 stub，直接加入 IDE include path 即可。

**Q4：`scripts/install.sh` 做了什么？**

1. 检查 Rust 工具链（cargo）和 php-config
2. 若 `target/release/liblychee_worker.so` 不存在则 `cargo build --release`
3. 把扩展文件拷贝到 `php-config --extension-dir`（必要时用 sudo）
4. 检测当前加载的 `php.ini` 并追加 `extension=lychee_worker`（若未检测到 ini 则写入 `conf.d/99-lychee_worker.ini`）
5. `php -m | grep lychee_worker` 做最终验证

支持 `--no-ini`（跳过 php.ini 写入）和 `--ini=/path/to/custom/php.ini`（指定 ini 路径）参数。

## 架构概览

```
+---------------- PHP 进程（父）-----------------------------------+
|  lychee_worker_start(...)                                       |
|  -> fork N 个 HTTP/WebSocket 子进程（php think worker:child）   |
|  -> fork 1 个 Queue 消费子进程（php think worker:queue，可选）  |
|  -> 监控 mtime，变化时 kill 所有子进程并重启                     |
|  -> 捕获 Ctrl-C，优雅关闭                                       |
+-----fork---------fork-------fork--------fork-------fork---------+
        |             |          |           |          |
   +-HTTP 1-+    +-HTTP 2-+ +-HTTP N-+   +-Queue-+     +-Cron-+
   | tokio  |    | tokio  | | tokio  |   | while|     |独立进程|
   | TcpLn  |    | TcpLn  | | TcpLn  |   | sleep|     |php lee|
   | DashMap|    | DashMap| | DashMap|   | queue|     |  cron |
   |callPHP|    |callPHP| |callPHP|   | PHP  |     |      |
   +-------+    +-------+ +-------+   +------+     +------+

  注意：每个 HTTP 子进程有独立的 CONNECTIONS / ROOMS / 统计计数器。
  当 SO_REUSEPORT 启用时，客户端连接会被操作系统随机分发到不同子进程，
  因此跨子进程广播当前需要上层 PHP 业务处理（例如 Redis 消息队列）。
```

## 控制台输出示例

```
Lychee Worker starting...
  Listen  : 0.0.0.0:8080
  Workers : 2
  Queue   : off
  Press Ctrl+C to stop.
```

启动后进入阻塞运行，直到收到 Ctrl-C 或 `lychee_worker_stop()`。

### worker:status 输出示例

在 ThinkPHP 项目根目录随时执行：

```bash
php think worker:status
```

会得到类似以下的输出（数值为当前 PHP 进程内的统计，memory 字段以 MB 展示）：

```
Lychee Worker — Runtime Status
------------------------------------------------
  connections : 3
  ws          : 1
  requests    : 42
  rooms       : 2
  memory      : 29.52 MB
------------------------------------------------
```

> memory 在 Linux 下从 `/proc/self/status` 的 `VmRSS` 读取，在 macOS 下通过 `proc_pidinfo(PROC_PIDTASKINFO)` 的 `pti_resident_size` 读取；其他平台暂不支持，对应字段会被省略。

## 配置（ThinkPHP 8）

ThinkPHP 安装时会从 `vendor/watsonhaw/lychee-worker/src/config/worker.php` 读取默认配置，可在项目 `.env` 中覆盖：

```dotenv
LYCHEE_HOST=127.0.0.1
LYCHEE_PORT=9090
LYCHEE_WORKERS=2
LYCHEE_QUEUE=true
LYCHEE_WATCH_DIRS=app,config,route
LYCHEE_WATCH_NAMES=*.php
LYCHEE_WATCH_EXCLUDES=runtime,vendor
LYCHEE_WATCH_INTERVAL=1000
LYCHEE_PING_INTERVAL=25
LYCHEE_PING_TIMEOUT=60
```

也可以在项目中新建 `config/worker.php`，以数组形式自定义默认值：

```php
return [
    'host'              => '127.0.0.1',
    'port'              => 9090,
    'worker_num'        => 2,
    'enable_queue'      => false,
    'watch_dirs'        => 'app,config,route',
    'watch_names'       => '*.php',
    'watch_excludes'    => 'runtime,vendor',
    'watch_interval_ms' => 1000,
    'ping_interval_sec' => 25,
    'ping_timeout_sec'  => 60,
];
```

`php think worker`（生产模式）会忽略 `watch_*` 相关配置以关闭热更新；`php think worker:dev`（开发模式）则会启用文件监控。

## 测试

项目包含两类测试：

| 类型 | 位置 | 覆盖内容 |
|---|---|---|
| **Rust 单元测试** | `rust/src/` 内联（通过 `cargo test --release` 执行） | WebSocket key 提取、Sec-WebSocket-Accept 计算、事件负载构造等 |
| **PHP 插件测试** | `tests/`（PHPUnit，通过 `composer test-php` 或 `vendor/bin/phpunit` 执行） | `ServiceTest` / `ManagerTest` / `WebsocketTest` / `ConfigFileTest` / `PluginDeclarationTest` — 验证 ThinkPHP 插件的命令注册、配置合并、Websocket 静态方法降级、`composer.json` 的 `extra.think.*` 声明正确性 |

本地运行：

```bash
# Rust 单元测试
cargo test --release -- --nocapture

# PHP 插件单元测试（无需 ThinkPHP 框架，测试自带最小 stub）
vendor/bin/phpunit

# 一次性运行所有测试
composer test
```

CI 已在 `.github/workflows/ci.yml`（分支/PR）与 `release.yml`（打 tag 时多平台 × PHP 8.3）里自动跑上述测试，并在 Release 产物中挂出各平台的预编译 `.so`/`.dylib`。

## 发布流程（维护者）

```bash
# 打 tag 并推送，release.yml 会自动在 Linux/macOS × PHP 8.3 矩阵中
# 构建扩展、跑 Rust + PHP 测试、然后把预编译产物挂到 GitHub Release。
git tag 0.1.0
git push origin 0.1.0
```

发布后：
- 用户执行 `pie install watsonhaw/lychee-worker`，**PIE 会优先下载预编译 ZIP**（免构建），缺失时才回退到源码构建
- 用户也可以通过 `composer require watsonhaw/lychee-worker` 拉取源码，再跑 `scripts/install.sh`（带 `--from-github-release=<tag>` 时同样可免构建）
- 用户也可以直接从 Release 页下载 ZIP 或 `liblychee_worker-<OS>-<arch>.so`，拷到 `php-config --extension-dir` 后在 `php.ini` 启用