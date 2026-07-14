//! 文件监控：手动触发热更新。

pub fn trigger_reload() -> bool {
    // 直接退出进程，由父进程重新 fork
    std::process::exit(0);
}
