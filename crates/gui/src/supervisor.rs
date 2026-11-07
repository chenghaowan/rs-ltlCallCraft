//! 插件化自动拉起:启动时探测 server,不可达且同目录有 `ltl-chat-server.exe`
//! 时拉起一个本地常驻实例(即装即用)。
//!
//! 规则:
//! - 仅对回环地址(127.0.0.1/localhost)代管;登录页填的中心服务器永不拉起。
//! - server 已可达(手动启动/上次拉起的常驻实例/别的 profile 拉起过)则直接复用。
//! - 拉起的 server 数据目录与 GUI 同基目录:`%APPDATA%/ltl-chat/server`,
//!   日志落 `server.log`,GUI 退出后 server 常驻(留档/附件继续可用)。

use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

/// server 地址是否指向本机(只对回环自动拉起)
fn is_loopback_server(server: &str) -> bool {
    let Some((host, _)) = host_port(server) else {
        return false;
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

/// 从 http(s)://host[:port][/..] 提取 host( scheme 判断在剥前缀前做 )
fn host_of(server: &str) -> &str {
    let rest = server
        .strip_prefix("https://")
        .or_else(|| server.strip_prefix("http://"))
        .unwrap_or(server);
    let end = rest.find('/').unwrap_or(rest.len());
    &rest[..end]
}

/// 从 server 地址提取 (host, port), 缺省端口按 scheme
fn host_port(server: &str) -> Option<(String, u16)> {
    let https = server.starts_with("https://");
    let authority = host_of(server);
    if authority.is_empty() {
        return None;
    }
    let default_port = |https| if https { 443 } else { 80 };
    // [::1]:8901 → IPv6 字面量
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = &rest[..end];
        if host.is_empty() {
            return None;
        }
        let port = rest[end + 1..]
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| default_port(https));
        return Some((host.to_string(), port));
    }
    // 裸 IPv6(多个冒号无方括号)整段当 host
    let (host, port) = if authority.matches(':').count() > 1 {
        (authority, default_port(https))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
                (h, p.parse::<u16>().ok()?)
            }
            _ => (authority, default_port(https)),
        }
    };
    if host.is_empty() {
        None
    } else {
        Some((host.to_string(), port))
    }
}

/// TCP 连通性探测(解析 + 连接 800ms 超时)
fn probe(server: &str) -> bool {
    let Some((host, port)) = host_port(server) else {
        return false;
    };
    let Ok(mut addrs) = (host.as_str(), port).to_socket_addrs() else {
        return false;
    };
    let Some(addr) = addrs.next() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(800)).is_ok()
}

/// 启动前确保回环 server 可达;不可达则拉起同目录的 ltl-chat-server.exe 并等待就绪(≤15s)。
pub fn ensure_local_server(server: &str) {
    if !is_loopback_server(server) {
        return; // 中心部署: 不代管
    }
    if probe(server) {
        tracing::debug!("server {server} 已可达, 复用现有实例");
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(dir) = exe.parent() else {
        return;
    };
    let server_exe = dir.join("ltl-chat-server.exe");
    if !server_exe.is_file() {
        tracing::info!(
            "server {server} 不可达且同目录无 ltl-chat-server.exe, 跳过自动拉起(开发请手动 cargo run -p ltl-chat-server)"
        );
        return;
    }
    // 数据目录与 GUI 同基目录(与 util::data_dir 对齐), 日志落 server.log
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ltl-chat")
        .join("server");
    let _ = std::fs::create_dir_all(&data_dir);

    let mut cmd = std::process::Command::new(&server_exe);
    cmd.args(["serve", "--data-dir"]).arg(&data_dir);
    if let Ok(log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join("server.log"))
    {
        cmd.stdout(std::process::Stdio::from(log.try_clone().expect("clone log")));
        cmd.stderr(std::process::Stdio::from(log));
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    match cmd.spawn() {
        Ok(child) => tracing::info!(
            "已拉起本地 ltl-chat-server (pid {:?}, data-dir {})",
            child.id(),
            data_dir.display()
        ),
        Err(e) => {
            tracing::warn!("自动拉起 ltl-chat-server 失败: {e}");
            return;
        }
    }
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(500));
        if probe(server) {
            tracing::info!("本地 ltl-chat-server 就绪: {server}");
            return;
        }
    }
    tracing::warn!("等待本地 ltl-chat-server 就绪超时({server}), 请检查端口占用或 server.log");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_cases() {
        assert_eq!(
            host_port("http://127.0.0.1:8901"),
            Some(("127.0.0.1".into(), 8901))
        );
        assert_eq!(host_port("http://127.0.0.1"), Some(("127.0.0.1".into(), 80)));
        assert_eq!(
            host_port("https://chat.example.com/api"),
            Some(("chat.example.com".into(), 443))
        );
        assert_eq!(host_port("http://"), None);
    }

    #[test]
    fn loopback_only() {
        assert!(is_loopback_server("http://127.0.0.1:8901"));
        assert!(is_loopback_server("http://LOCALHOST:8901"));
        assert!(!is_loopback_server("http://192.168.1.10:8901"));
    }
}
