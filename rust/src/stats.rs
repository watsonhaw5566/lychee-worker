//! 运行时统计聚合。

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use crate::runtime::{CONNECTIONS, REQ_COUNT, ROOMS, WS_COUNT};

pub fn snapshot() -> HashMap<String, i64> {
    let mut map = HashMap::new();
    let conn_len = CONNECTIONS.with(|map| map.len()) as i64;
    let room_len = ROOMS.with(|map| map.len()) as i64;
    map.insert("connections".into(), conn_len);
    map.insert("ws".into(), WS_COUNT.load(Ordering::SeqCst));
    map.insert("requests".into(), REQ_COUNT.load(Ordering::SeqCst));
    map.insert("rooms".into(), room_len);

    // 进程物理内存占用（单位：KB）
    if let Some(kb) = current_rss_kb() {
        map.insert("memory_rss_kb".into(), kb);
    }

    map
}

/// 返回当前进程物理内存占用（RSS，单位：KB）。
///
/// - Linux：解析 `/proc/self/status` 的 `VmRSS`。
/// - macOS：通过 `proc_pidinfo(PROC_PIDTASKINFO)` 拿 `pti_resident_size` 再换算。
/// - 其他平台：返回 `None`。
fn current_rss_kb() -> Option<i64> {
    #[cfg(target_os = "linux")]
    {
        rss_linux_kb()
    }
    #[cfg(target_os = "macos")]
    {
        rss_macos_bytes().map(|b| b / 1024)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn rss_linux_kb() -> Option<i64> {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let file = File::open("/proc/self/status").ok()?;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            // 形如 "VmRSS:\t   123456 kB"
            let num_str = rest
                .trim()
                .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c.is_whitespace());
            return num_str.trim().parse::<i64>().ok();
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn rss_macos_bytes() -> Option<i64> {
    // 使用 XNU 的 `proc_pidinfo(PROC_PIDTASKINFO)` 直接读取当前进程的 resident_size。
    // 比 Mach `task_info(MACH_TASK_BASIC_INFO)` 更稳、不需要额外权限。
    // 相关类型参考：
    //   https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h
    #[allow(non_camel_case_types)]
    type pid_t = i32;
    const PROC_PIDTASKINFO: i32 = 4;

    // 注意字段顺序与内核 ABI 必须一致：字段都是 64 位整数，`proc_taskinfo` 总大小 168 字节。
    // 只需要第二个字段 `pti_resident_size`，因此后面的字段用占位字节数盖住即可。
    #[repr(C)]
    struct ProcTaskinfoLeading {
        _pti_virtual_size: u64,
        pti_resident_size: u64,
        _rest: [u8; 168 - 16],
    }

    impl Default for ProcTaskinfoLeading {
        fn default() -> Self {
            // 直接清零整个结构体。unsafe 块仅用于 mem::zeroed()。
            unsafe { std::mem::zeroed() }
        }
    }

    extern "C" {
        fn proc_pidinfo(
            pid: pid_t,
            flavor: i32,
            arg: u64,
            buffer: *mut std::ffi::c_void,
            buffersize: i32,
        ) -> i32;
        fn getpid() -> pid_t;
    }

    let mut info = ProcTaskinfoLeading::default();
    let size = std::mem::size_of::<ProcTaskinfoLeading>() as i32;
    let pid = unsafe { getpid() };

    let ret = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDTASKINFO,
            0,
            (&mut info as *mut ProcTaskinfoLeading).cast::<std::ffi::c_void>(),
            size,
        )
    };

    // 返回值 == 写入的字节数；> 0 表示成功。
    if ret > 0 && info.pti_resident_size > 0 {
        Some(info.pti_resident_size as i64)
    } else {
        None
    }
}