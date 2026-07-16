//! SSH SOCKS tunnel lifecycle for the ProxySwitch desktop UI.

use crate::config::get_app_config_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const LOCAL_SOCKS_HOST: &str = "127.0.0.1";
const LOCAL_SOCKS_PORT: u16 = 7890;
const REMOTE_HOST: &str = "154.21.84.35";
const REMOTE_PORT: u16 = 12581;
const REMOTE_USER: &str = "root";
const TUNNEL_WAIT_ATTEMPTS: usize = 30;
const PROXY_URL: &str = "socks5://127.0.0.1:7890";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySwitchStatus {
    pub state: String,
    pub tunnel_running: bool,
    pub port_listening: bool,
    pub pid: Option<u32>,
    pub local_socks: String,
    pub remote_ssh: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySwitchDiagnostic {
    pub tunnel: ProxySwitchStatus,
    pub github_reachable: bool,
    pub latency_ms: u64,
    pub git_proxy_configured: bool,
    pub git_proxy_matches_tunnel: bool,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TunnelRecord {
    pid: u32,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct ProxySwitchPreferences {
    auto_connect_on_startup: bool,
}

fn record_path() -> PathBuf {
    get_app_config_dir().join("proxyswitch-tunnel.json")
}

fn preferences_path() -> PathBuf {
    get_app_config_dir().join("proxyswitch-preferences.json")
}

fn read_preferences() -> ProxySwitchPreferences {
    fs::read_to_string(preferences_path())
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn write_preferences(preferences: &ProxySwitchPreferences) -> Result<(), String> {
    let path = preferences_path();
    let parent = path
        .parent()
        .ok_or_else(|| "无法确定代理设置目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建代理设置目录失败: {error}"))?;
    let content = serde_json::to_string(preferences).map_err(|error| error.to_string())?;
    fs::write(path, content).map_err(|error| format!("保存自动连接设置失败: {error}"))
}

pub fn is_proxyswitch_auto_connect_enabled() -> bool {
    read_preferences().auto_connect_on_startup
}

fn socks_addr() -> Result<SocketAddr, String> {
    format!("{LOCAL_SOCKS_HOST}:{LOCAL_SOCKS_PORT}")
        .parse::<SocketAddr>()
        .map_err(|error| format!("无效的本地 SOCKS 地址: {error}"))
}

fn is_port_open() -> bool {
    socks_addr()
        .map(|address| TcpStream::connect_timeout(&address, Duration::from_millis(150)).is_ok())
        .unwrap_or(false)
}

fn read_record() -> Option<TunnelRecord> {
    let content = fs::read_to_string(record_path()).ok()?;
    serde_json::from_str(&content).ok()
}

fn write_record(record: &TunnelRecord) -> Result<(), String> {
    let path = record_path();
    let parent = path
        .parent()
        .ok_or_else(|| "无法确定代理状态目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建代理状态目录失败: {error}"))?;
    let content = serde_json::to_string(record).map_err(|error| error.to_string())?;
    fs::write(path, content).map_err(|error| format!("保存代理状态失败: {error}"))
}

fn remove_record() {
    if let Err(error) = fs::remove_file(record_path()) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("删除代理状态文件失败: {error}");
        }
    }
}

#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_process_running(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

#[cfg(unix)]
fn is_our_tunnel_process(pid: u32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .map(|output| {
            let command = String::from_utf8_lossy(&output.stdout);
            command.contains("ssh")
                && command.contains("-D")
                && command.contains(&format!("{LOCAL_SOCKS_HOST}:{LOCAL_SOCKS_PORT}"))
        })
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_our_tunnel_process(pid: u32) -> bool {
    is_process_running(pid)
}

fn status_from_record(record: Option<TunnelRecord>) -> ProxySwitchStatus {
    let port_listening = is_port_open();
    let (pid, process_running, last_error) = match record {
        Some(record) if is_process_running(record.pid) => {
            let last_error = (!port_listening)
                .then(|| "SSH 进程仍在运行，但本地 SOCKS 端口未监听。".to_string());
            (Some(record.pid), true, last_error)
        }
        Some(_) => {
            remove_record();
            (None, false, None)
        }
        None => (None, false, None),
    };

    let tunnel_running = process_running && port_listening;
    let state = if tunnel_running {
        "on"
    } else if process_running || port_listening {
        "error"
    } else {
        "off"
    };
    let last_error = last_error.or_else(|| {
        (!process_running && port_listening)
            .then(|| "本地 7890 端口已被其他进程占用，ProxySwitch 不会接管它。".to_string())
    });

    ProxySwitchStatus {
        state: state.to_string(),
        tunnel_running,
        port_listening,
        pid,
        local_socks: format!("{LOCAL_SOCKS_HOST}:{LOCAL_SOCKS_PORT}"),
        remote_ssh: format!("{REMOTE_USER}@{REMOTE_HOST}:{REMOTE_PORT}"),
        last_error,
    }
}

fn git_proxy_matches_tunnel() -> (bool, bool) {
    let values: Vec<String> = ["http.proxy", "https.proxy"]
        .iter()
        .filter_map(|key| {
            Command::new("git")
                .args(["config", "--global", "--get", key])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        })
        .filter(|value| !value.is_empty())
        .collect();

    let configured = !values.is_empty();
    let expected = format!("{LOCAL_SOCKS_HOST}:{LOCAL_SOCKS_PORT}");
    let matches_tunnel = values.iter().any(|value| value.contains(&expected));
    (configured, matches_tunnel)
}

/// Return the tunnel process and local listener state without changing it.
#[tauri::command]
pub fn get_proxyswitch_status() -> ProxySwitchStatus {
    status_from_record(read_record())
}

#[tauri::command]
pub fn get_proxyswitch_auto_connect() -> bool {
    is_proxyswitch_auto_connect_enabled()
}

#[tauri::command]
pub fn set_proxyswitch_auto_connect(enabled: bool) -> Result<bool, String> {
    let preferences = ProxySwitchPreferences {
        auto_connect_on_startup: enabled,
    };
    write_preferences(&preferences)?;
    Ok(enabled)
}

/// Check the SSH tunnel, GitHub reachability through it, and Git proxy alignment.
#[tauri::command]
pub async fn diagnose_proxyswitch() -> ProxySwitchDiagnostic {
    let tunnel = status_from_record(read_record());
    let (git_proxy_configured, git_proxy_matches_tunnel) = git_proxy_matches_tunnel();

    if !tunnel.tunnel_running {
        return ProxySwitchDiagnostic {
            tunnel,
            github_reachable: false,
            latency_ms: 0,
            git_proxy_configured,
            git_proxy_matches_tunnel,
            error: Some("SSH SOCKS 隧道未运行，已跳过 GitHub 连通性检测。".to_string()),
        };
    }

    let start = Instant::now();
    let result = reqwest::Proxy::all(PROXY_URL)
        .map_err(|error| format!("创建 SOCKS 代理失败: {error}"))
        .and_then(|proxy| {
            reqwest::Client::builder()
                .proxy(proxy)
                .user_agent("ProxySwitch/1.0")
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(15))
                .build()
                .map_err(|error| format!("创建诊断客户端失败: {error}"))
        });

    let (github_reachable, error) = match result {
        Ok(client) => match client.get("https://api.github.com/rate_limit").send().await {
            Ok(_) => (true, None),
            Err(error) => (false, Some(format!("GitHub 连通性检测失败: {error}"))),
        },
        Err(error) => (false, Some(error)),
    };

    ProxySwitchDiagnostic {
        tunnel,
        github_reachable,
        latency_ms: start.elapsed().as_millis() as u64,
        git_proxy_configured,
        git_proxy_matches_tunnel,
        error,
    }
}

/// Start an SSH dynamic forward using the user's normal SSH config and keys.
#[tauri::command]
pub fn start_proxyswitch_tunnel() -> Result<ProxySwitchStatus, String> {
    let current = status_from_record(read_record());
    if current.tunnel_running {
        return Ok(current);
    }
    if current.port_listening {
        return Err(
            "本地 127.0.0.1:7890 已被其他进程占用，无法启动 ProxySwitch 隧道。".to_string(),
        );
    }

    let mut child = Command::new("ssh")
        .args([
            "-N",
            "-D",
            &format!("{LOCAL_SOCKS_HOST}:{LOCAL_SOCKS_PORT}"),
            "-p",
            &REMOTE_PORT.to_string(),
            "-o",
            "BatchMode=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=30",
            "-o",
            "ServerAliveCountMax=3",
            &format!("{REMOTE_USER}@{REMOTE_HOST}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("启动 SSH 隧道失败: {error}"))?;
    let pid = child.id();

    for _ in 0..TUNNEL_WAIT_ATTEMPTS {
        if is_port_open() {
            let record = TunnelRecord { pid };
            if let Err(error) = write_record(&record) {
                let _ = child.kill();
                return Err(error);
            }
            return Ok(status_from_record(Some(record)));
        }
        if let Ok(Some(exit_status)) = child.try_wait() {
            return Err(format!("SSH 隧道启动失败，进程退出状态: {exit_status}"));
        }
        thread::sleep(Duration::from_millis(200));
    }

    let _ = child.kill();
    Err(
        "SSH 隧道未能在 6 秒内开始监听 127.0.0.1:7890。请检查 SSH 密钥、服务器地址和网络。"
            .to_string(),
    )
}

/// Stop only the SSH process that was previously started by ProxySwitch.
#[tauri::command]
pub fn stop_proxyswitch_tunnel() -> Result<ProxySwitchStatus, String> {
    let Some(record) = read_record() else {
        return Ok(status_from_record(None));
    };

    if !is_process_running(record.pid) {
        remove_record();
        return Ok(status_from_record(None));
    }
    if !is_our_tunnel_process(record.pid) {
        return Err(
            "记录的进程不再是 ProxySwitch SSH 隧道，已拒绝停止以避免影响其他程序。".to_string(),
        );
    }

    #[cfg(unix)]
    let stopped = Command::new("kill")
        .args(["-TERM", &record.pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    #[cfg(windows)]
    let stopped = Command::new("taskkill")
        .args(["/PID", &record.pid.to_string(), "/T", "/F"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if !stopped {
        return Err("停止 SSH 隧道失败。".to_string());
    }

    for _ in 0..10 {
        if !is_process_running(record.pid) {
            remove_record();
            return Ok(status_from_record(None));
        }
        thread::sleep(Duration::from_millis(100));
    }

    Err("SSH 隧道进程没有在预期时间内退出。".to_string())
}
