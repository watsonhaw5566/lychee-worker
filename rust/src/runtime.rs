//! Prefork 多进程运行时。
//!
//! 进程模型：
//!   父进程 -> fork N 个 HTTP/WebSocket 子进程 + 1 个 Queue 子进程 -> 监控文件 mtime
//!   HTTP 子进程 -> 跑独立 tokio runtime -> 监听端口、处理 HTTP/WebSocket
//!   Queue 子进程 -> 跑纯 PHP 循环（php think worker:queue）消费队列任务
//!

use dashmap::DashMap;
use ext_php_rs::types::ZendCallable;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;

pub struct WorkerConfig {
    pub host: String,
    pub port: u16,
    pub worker_num: usize,
    pub watch_dirs: Vec<String>,
    pub watch_names: Vec<String>,
    pub watch_excludes: Vec<String>,
    pub watch_interval_ms: u64,
    pub ping_interval_sec: u64,
    pub ping_timeout_sec: u64,
    pub enable_queue: bool,
    /// 单个请求总超时（秒）：header/body/PHP 回调/响应写入
    pub request_timeout_sec: u64,
    /// 每个子进程最大并发连接数（超限直接 503）
    pub max_connections: usize,
    /// 请求 header 最大字节数
    pub header_max_bytes: usize,
    /// 请求 body 最大字节数（Content-Length 超限直接 413）
    pub body_max_bytes: usize,
}

/// 进程全局连接表：ConnId -> tokio mpsc::UnboundedSender
pub static CONNECTIONS: GlobalMap<DashMap<String, SenderCell>> = GlobalMap::new();

/// 进程全局房间表
pub static ROOMS: GlobalMap<DashMap<String, RoomCell>> = GlobalMap::new();

/// 请求计数
pub static REQ_COUNT: AtomicI64 = AtomicI64::new(0);
pub static WS_COUNT: AtomicI64 = AtomicI64::new(0);
/// 当前活跃 HTTP 连接数（用于 max_connections 限流）
pub static ACTIVE_HTTP_CONNS: AtomicI64 = AtomicI64::new(0);

/// 全局停止标志
static STOP_FLAG: AtomicBool = AtomicBool::new(false);

/// PHP 调用全局互斥锁。
///
/// PHP 解释器（非 ZTS 构建）在同一进程内不能被多个线程并发调用，
/// 否则会出现内存损坏。所有对 `ZendCallable::try_call` / ext-php-rs
/// 的 PHP 回调都必须通过此锁串行化。
static PHP_CALL_LOCK: Mutex<()> = Mutex::new(());

/// 在 tokio 异步上下文中安全地执行一次 PHP 同步回调。
///
/// 工作方式：
///   1. `tokio::task::block_in_place` —— 告诉 tokio 把当前线程临时
///      退出 reactor 角色，由其他 worker 线程继续接管其他连接的
///      I/O（accept、WebSocket ping/pong、SSE 写入等）。
///   2. `PHP_CALL_LOCK.lock()` —— 同一时刻只允许一个线程进入 PHP
///      解释器（非 ZTS 的硬性约束）。
///
/// 为什么不用 `spawn_blocking`？
///   • `ext-php-rs` 的 `ZendCallable` 不是 `Send`，无法被送到
///     另一个线程去调用；
///   • `block_in_place` 在当前线程执行，PHP 仍在此线程被调用，
///     但 tokio 会把此线程上其他 pending 的 reactor 工作迁移到
///     其他 worker 线程。
pub fn run_php_blocking<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    tokio::task::block_in_place(|| {
        let _guard = PHP_CALL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f()
    })
}

// 下面是实现细节：把 DashMap 封装成一个可控初始化的全局静态结构
// （使用 OnceLock + 内部分配，避免复杂的 lazy_static）

use std::sync::OnceLock;

pub struct GlobalMap<T>(OnceLock<T>);

impl<T> GlobalMap<T> {
    pub const fn new() -> Self {
        Self(OnceLock::new())
    }

    fn init_default(&self)
    where
        T: Default,
    {
        // 若未初始化则初始化
        if self.0.get().is_none() {
            let _ = self.0.set(T::default());
        }
    }
}

impl<T: Default + Send + Sync> GlobalMap<T> {
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.init_default();
        f(self.0.get().unwrap())
    }
}

// Sender 的包装类型（避免在 tokio 初始化前分配）
pub struct SenderCell {
    pub sender: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

// Room 成员集合
pub struct RoomCell {
    pub members: std::sync::Mutex<Vec<String>>,
}

impl RoomCell {
    pub fn new() -> Self {
        Self {
            members: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl Default for RoomCell {
    fn default() -> Self {
        Self::new()
    }
}

/// 子进程类型标识（用于重启时区分）
#[derive(Clone, Copy)]
enum ChildKind {
    Http,
    Queue,
}

struct TrackedChild {
    kind: ChildKind,
    child: std::process::Child,
}

pub struct WorkerRuntime;

impl WorkerRuntime {
    /// 阻塞入口：PHP 侧 lychee_worker_start 调用
    pub fn run_blocking<'a>(
        cfg: WorkerConfig,
        http_handler: Option<&'a ZendCallable<'a>>,
        ws_open_handler: Option<&'a ZendCallable<'a>>,
        ws_message_handler: Option<&'a ZendCallable<'a>>,
        ws_close_handler: Option<&'a ZendCallable<'a>>,
    ) -> Result<(), String> {
        // HTTP 子进程模式：LYCHEE_WORKER_CHILD=1（进入 Rust tokio 事件循环）
        if std::env::var("LYCHEE_WORKER_CHILD").ok().as_deref() == Some("1") {
            return Self::run_child_tokio(
                cfg,
                http_handler,
                ws_open_handler,
                ws_message_handler,
                ws_close_handler,
            );
        }
        // 父进程模式（不使用传入的回调，只 fork 子进程）
        // 在启动前探测端口占用；若被占用则给出命令提示并直接退出，
        // 避免子进程进入无限重启循环。
        if let Err(msg) = crate::http::probe_port(&cfg.host, cfg.port) {
            eprintln!("{}", msg);
            return Err("port in use".to_string());
        }

        let children = std::sync::Arc::new(std::sync::Mutex::new(Vec::<TrackedChild>::new()));

        // 启动 HTTP/WebSocket 子进程
        let mut http_started = 0;
        for _ in 0..cfg.worker_num {
            if let Ok(child) = spawn_http_child(&cfg) {
                children.lock().unwrap().push(TrackedChild {
                    kind: ChildKind::Http,
                    child,
                });
                http_started += 1;
            }
        }
        if http_started > 0 {
            println!(
                "{:<10} {:<30} {:<8} [OK]",
                "tcp",
                format!("http://{}:{}", cfg.host, cfg.port),
                1 + http_started,
            );
        }

        // 启动 Queue worker（独立进程，跑 PHP 消费循环）
        let mut queue_started = 0;
        if cfg.enable_queue {
            if let Ok(child) = spawn_queue_child(&cfg) {
                children.lock().unwrap().push(TrackedChild {
                    kind: ChildKind::Queue,
                    child,
                });
                queue_started += 1;
            }
        }
        if queue_started > 0 {
            println!(
                "{:<10} {:<30} {:<8} [OK]",
                "unix",
                "queue worker",
                1 + queue_started,
            );
        }

        // 底部提示
        println!("{}", str::repeat("-", 70));
        println!("Press Ctrl+C to stop.");

        // Ctrl-C 捕获
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_c = stop.clone();
        let _ = ctrlc::set_handler(move || {
            stop_c.store(true, Ordering::SeqCst);
        });

        // 监控目录的 mtime（带文件名模式匹配和 exclude 支持）
        let mut prev_mtime = HashMap::new();
        for dir in &cfg.watch_dirs {
            scan_mtime(
                std::path::Path::new(dir),
                &cfg.watch_names,
                &cfg.watch_excludes,
                &mut prev_mtime,
            );
        }

        loop {
            if stop.load(Ordering::SeqCst) {
                kill_all_children(&children);
                return Ok(());
            }
            reap_and_restart(&children, &cfg);

            // 检查是否有文件变更
            let mut cur_mtime = HashMap::new();
            for dir in &cfg.watch_dirs {
                scan_mtime(
                    std::path::Path::new(dir),
                    &cfg.watch_names,
                    &cfg.watch_excludes,
                    &mut cur_mtime,
                );
            }
            let changed = cur_mtime.len() != prev_mtime.len()
                || cur_mtime.iter().any(|(k, v)| prev_mtime.get(k) != Some(v));
            if changed {
                eprintln!("[lychee-worker] file changed, restarting");
                kill_all_children(&children);
                for _ in 0..cfg.worker_num {
                    if let Ok(child) = spawn_http_child(&cfg) {
                        children.lock().unwrap().push(TrackedChild {
                            kind: ChildKind::Http,
                            child,
                        });
                    }
                }
                if cfg.enable_queue {
                    if let Ok(child) = spawn_queue_child(&cfg) {
                        children.lock().unwrap().push(TrackedChild {
                            kind: ChildKind::Queue,
                            child,
                        });
                    }
                }
                prev_mtime = cur_mtime;
            }
            std::thread::sleep(std::time::Duration::from_millis(cfg.watch_interval_ms));
        }
    }

    /// 子进程：跑 tokio runtime（由 spawn_one_child fork 后调用）
    pub fn run_child_tokio<'a>(
        cfg: WorkerConfig,
        http_handler: Option<&'a ZendCallable<'a>>,
        ws_open_handler: Option<&'a ZendCallable<'a>>,
        ws_message_handler: Option<&'a ZendCallable<'a>>,
        ws_close_handler: Option<&'a ZendCallable<'a>>,
    ) -> Result<(), String> {
        // 使用 multi_thread runtime：当某个任务在 `block_in_place` 中被
        // PHP 阻塞时，tokio 会把其他 pending task 调度到另一条 worker
        // 线程，从而保证该子进程的其他连接（WebSocket 心跳、新 accept、
        // SSE 写入等）不被阻塞。
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(8)
            .enable_all()
            .build()
            .map_err(|e| format!("tokio build: {}", e))?;
        rt.block_on(async move {
            crate::http::serve(
                cfg.host.clone(),
                cfg.port,
                http_handler,
                ws_open_handler,
                ws_message_handler,
                ws_close_handler,
                &cfg,
            )
            .await
            .map_err(|e| format!("http serve: {}", e))
        })
    }

    /// 优雅停止：设置停止标志
    pub fn stop() {
        STOP_FLAG.store(true, Ordering::SeqCst);
    }
}

fn spawn_php_subcommand(
    script: &str,
    subcmd: &str,
    cfg: &WorkerConfig,
    env_key: &str,
    pass_server_args: bool,
) -> std::io::Result<std::process::Child> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new("php");
    cmd.arg(script).arg(subcmd);
    if pass_server_args {
        cmd.arg("--host")
            .arg(&cfg.host)
            .arg("--port")
            .arg(cfg.port.to_string())
            .arg("--worker-num")
            .arg(cfg.worker_num.to_string());
    }
    cmd.env(env_key, "1")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
}

fn spawn_http_child(cfg: &WorkerConfig) -> std::io::Result<std::process::Child> {
    let script = std::env::var("LYCHEE_WORKER_ENTRY").unwrap_or_else(|_| "think".into());
    spawn_php_subcommand(&script, "worker:child", cfg, "LYCHEE_WORKER_CHILD", true)
}

fn spawn_queue_child(cfg: &WorkerConfig) -> std::io::Result<std::process::Child> {
    let script = std::env::var("LYCHEE_WORKER_ENTRY").unwrap_or_else(|_| "think".into());
    // queue 消费者只跑 PHP 消费循环，不需要 host/port/worker-num 参数
    // （`worker:queue` 命令也未声明这些 option，传了反而会报 RuntimeException）
    spawn_php_subcommand(&script, "worker:queue", cfg, "LYCHEE_WORKER_QUEUE", false)
}

fn kill_all_children(children: &std::sync::Arc<std::sync::Mutex<Vec<TrackedChild>>>) {
    let mut guard = children.lock().unwrap();
    for tracked in guard.iter_mut() {
        let _ = tracked.child.kill();
        let _ = tracked.child.wait();
    }
    guard.clear();
}

fn reap_and_restart(
    children: &std::sync::Arc<std::sync::Mutex<Vec<TrackedChild>>>,
    cfg: &WorkerConfig,
) {
    let mut guard = children.lock().unwrap();

    // 手动扫描：因为 Vec::retain 只给 &T，无法调用 Child::try_wait(&mut self)
    let mut kept: Vec<TrackedChild> = Vec::with_capacity(guard.len());
    let mut to_restart: Vec<ChildKind> = Vec::new();

    for tracked in std::mem::take(&mut *guard) {
        let mut child = tracked.child;
        match child.try_wait() {
            Ok(Some(_)) => {
                to_restart.push(tracked.kind);
            }
            _ => {
                kept.push(TrackedChild {
                    kind: tracked.kind,
                    child,
                });
            }
        }
    }

    *guard = kept;

    for kind in to_restart {
        let spawn_result = match kind {
            ChildKind::Http => spawn_http_child(cfg).map(|c| TrackedChild { kind, child: c }),
            ChildKind::Queue => spawn_queue_child(cfg).map(|c| TrackedChild { kind, child: c }),
        };
        if let Ok(tracked) = spawn_result {
            let label = match kind {
                ChildKind::Http => "http",
                ChildKind::Queue => "queue",
            };
            println!(
                "[lychee-worker] {} worker restarted (pid={})",
                label,
                tracked.child.id()
            );
            guard.push(tracked);
        }
    }
}

fn scan_mtime(
    dir: &std::path::Path,
    names: &[String],
    excludes: &[String],
    out: &mut HashMap<String, std::time::SystemTime>,
) {
    if !dir.exists() {
        return;
    }
    // 检查 exclude：如果目录路径包含任何 exclude 子串则跳过
    let dir_str = dir.to_string_lossy().to_string();
    for ex in excludes {
        if !ex.is_empty() && dir_str.contains(ex) {
            return;
        }
    }
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_mtime(&path, names, excludes, out);
            } else {
                // 文件名模式匹配（例如 *.php）
                let file_name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !names.is_empty() && !names.iter().any(|p| matches_pattern(&file_name, p)) {
                    continue;
                }
                if let Ok(meta) = path.metadata() {
                    if let Ok(mt) = meta.modified() {
                        out.insert(path.display().to_string(), mt);
                    }
                }
            }
        }
    }
}

/// 简单的 glob 模式匹配：支持 `*` 通配符，例如 `*.php`
fn matches_pattern(name: &str, pattern: &str) -> bool {
    // 将 pattern 按 '*' 切片，顺序匹配
    // 例如 "*.php" -> ["", ".php"]，name 必须以 ".php" 结尾
    // 例如 "app*.php" -> ["app", ".php"]，name 必须以 "app" 开头且以 ".php" 结尾
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.is_empty() {
        return name.is_empty();
    }
    // 处理开头：如果 pattern 不以 * 开头，则 name 必须以 parts[0] 开头
    if !pattern.starts_with('*') {
        if let Some(first) = parts.first() {
            if !name.starts_with(first) {
                return false;
            }
        }
    }
    // 处理结尾：如果 pattern 不以 * 结尾，则 name 必须以 parts.last() 结尾
    if !pattern.ends_with('*') {
        if let Some(last) = parts.last() {
            if !name.ends_with(last) {
                return false;
            }
        }
    }
    // 处理中间部分：每个切片必须按顺序在剩余字符串中出现
    if parts.len() > 2 {
        let middle = &parts[1..parts.len() - 1];
        let mut cursor = 0usize;
        for part in middle {
            if let Some(idx) = name[cursor..].find(part) {
                cursor += idx + part.len();
            } else {
                return false;
            }
        }
    }
    true
}
