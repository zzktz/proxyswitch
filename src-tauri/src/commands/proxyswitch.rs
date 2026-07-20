use crate::auto_launch;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const HOST: &str = "127.0.0.1";
const PORT: u16 = 7890;
const SSH_HOST: &str = "154.21.84.35";
const SSH_PORT: u16 = 12581;
const SSH_USER: &str = "root";
const SSH_IDENTITY_FILE: &str = ".ssh/154.21.84.35_ed25519";

// A manual stop must win over the one-shot startup auto-connect task.
static MANUAL_STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static TUNNEL_OPERATION: Mutex<()> = Mutex::new(());

#[derive(Default, Deserialize, Serialize)]
struct Preferences {
    auto_connect: bool,
    proxy_enabled: bool,
    pid: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: String,
    pub tunnel_running: bool,
    pub port_listening: bool,
    pub proxy_enabled: bool,
    pub auto_connect: bool,
    pub last_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub github_reachable: bool,
    pub latency_ms: u64,
    pub git_proxy_configured: bool,
    pub git_proxy_matches_tunnel: bool,
    pub error: Option<String>,
}

fn file() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ProxySwitch")
        .join("settings.json")
}
fn read() -> Preferences {
    fs::read_to_string(file())
        .ok()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}
fn save(value: &Preferences) -> Result<(), String> {
    let path = file();
    fs::create_dir_all(path.parent().ok_or("invalid settings path")?).map_err(|e| e.to_string())?;
    fs::write(path, serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn port_open() -> bool {
    format!("{HOST}:{PORT}")
        .parse::<SocketAddr>()
        .ok()
        .map(|a| TcpStream::connect_timeout(&a, Duration::from_millis(150)).is_ok())
        .unwrap_or(false)
}
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
#[cfg(windows)]
fn alive(_pid: u32) -> bool {
    false
}
#[cfg(unix)]
fn matching_tunnel(pid: u32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .map(|output| {
            let command = String::from_utf8_lossy(&output.stdout);
            command.contains("ssh")
                && command.contains(&format!("{HOST}:{PORT}"))
                && command.contains(&format!("{SSH_USER}@{SSH_HOST}"))
                && command.contains(&SSH_PORT.to_string())
                && command.contains("PROXYSWITCH_TUNNEL=1")
        })
        .unwrap_or(false)
}
#[cfg(windows)]
fn matching_tunnel(_pid: u32) -> bool {
    false
}
fn discover_existing_tunnel() -> Option<u32> {
    Command::new("lsof")
        .args(["-nP", "-t", &format!("-iTCP:{PORT}"), "-sTCP:LISTEN"])
        .output()
        .ok()
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()?
                .parse()
                .ok()
        })
        .filter(|pid| matching_tunnel(*pid))
}
fn status() -> Status {
    let mut p = read();
    if p.pid.is_none() && port_open() {
        p.pid = discover_existing_tunnel();
        let _ = save(&p);
    }
    let process = p.pid.map(alive).unwrap_or(false);
    if !process {
        p.pid = None;
        let _ = save(&p);
    }
    let listening = port_open();
    let running = process && listening;
    let error = if p.proxy_enabled && !running {
        Some("代理已启用但 SSH SOCKS 隧道未运行。".to_string())
    } else if listening && !process {
        Some("7890 端口已被其他进程占用。".to_string())
    } else {
        None
    };
    Status {
        state: if running && p.proxy_enabled {
            "on"
        } else if p.proxy_enabled && error.is_some() {
            "error"
        } else {
            "off"
        }
        .into(),
        tunnel_running: running,
        port_listening: listening,
        proxy_enabled: p.proxy_enabled,
        auto_connect: p.auto_connect,
        last_error: error,
    }
}
fn start() -> Result<(), String> {
    if status().tunnel_running {
        return Ok(());
    }
    if port_open() {
        return Err("127.0.0.1:7890 已被其他进程占用。".into());
    }
    let mut args = vec![
        "-N".to_string(),
        "-D".to_string(),
        format!("{HOST}:{PORT}"),
        "-p".to_string(),
        SSH_PORT.to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ExitOnForwardFailure=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=10".to_string(),
        "-o".to_string(),
        "SetEnv=PROXYSWITCH_TUNNEL=1".to_string(),
    ];
    if let Some(identity_path) = dirs::home_dir()
        .map(|home| home.join(SSH_IDENTITY_FILE))
        .filter(|path| path.is_file())
    {
        args.extend([
            "-i".to_string(),
            identity_path.to_string_lossy().into_owned(),
            "-o".to_string(),
            "IdentitiesOnly=yes".to_string(),
        ]);
    }
    args.push(format!("{SSH_USER}@{SSH_HOST}"));

    let mut child = Command::new("ssh")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("启动 SSH 隧道失败: {e}"))?;
    for _ in 0..30 {
        if port_open() {
            let mut p = read();
            p.pid = Some(child.id());
            save(&p)?;
            return Ok(());
        }
        if child.try_wait().ok().flatten().is_some() {
            return Err("SSH 隧道启动失败，请检查密钥和服务器连接。".into());
        }
        thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    Err("SSH 隧道未在 6 秒内监听本地端口。".into())
}
fn stop() -> Result<(), String> {
    let mut p = read();
    if let Some(pid) = p.pid.filter(|id| alive(*id)) {
        #[cfg(unix)]
        let terminated = Command::new("/bin/kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .map_err(|e| format!("无法停止 SSH 隧道: {e}"))?;
        #[cfg(unix)]
        if !terminated.success() {
            return Err("无法停止 SSH 隧道进程。".into());
        }
        #[cfg(unix)]
        for _ in 0..20 {
            if !alive(pid) {
                p.pid = None;
                return save(&p);
            }
            thread::sleep(Duration::from_millis(100));
        }
        #[cfg(unix)]
        let killed = Command::new("/bin/kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .map_err(|e| format!("无法强制停止 SSH 隧道: {e}"))?;
        #[cfg(unix)]
        if !killed.success() || alive(pid) {
            return Err("SSH 隧道未能停止。".into());
        }
    }
    p.pid = None;
    save(&p)
}
#[tauri::command]
pub fn get_proxyswitch_status() -> Status {
    status()
}
#[tauri::command]
pub fn set_auto_launch(enabled: bool) -> Result<bool, String> {
    auto_launch::set(enabled)?;
    Ok(enabled)
}
#[tauri::command]
pub fn get_auto_launch_status() -> Result<bool, String> {
    auto_launch::get()
}
#[tauri::command]
pub fn set_proxyswitch_auto_connect(enabled: bool) -> Result<bool, String> {
    let mut p = read();
    p.auto_connect = enabled;
    save(&p)?;
    Ok(enabled)
}
#[tauri::command]
pub fn enable_proxyswitch() -> Result<Status, String> {
    MANUAL_STOP_REQUESTED.store(false, Ordering::SeqCst);
    let _operation = TUNNEL_OPERATION
        .lock()
        .map_err(|_| "代理操作锁异常。".to_string())?;
    start()?;
    let mut p = read();
    p.proxy_enabled = true;
    save(&p)?;
    Ok(status())
}
#[tauri::command]
pub fn disable_proxyswitch() -> Result<Status, String> {
    MANUAL_STOP_REQUESTED.store(true, Ordering::SeqCst);
    let _operation = TUNNEL_OPERATION
        .lock()
        .map_err(|_| "代理操作锁异常。".to_string())?;
    let mut p = read();
    p.proxy_enabled = false;
    save(&p)?;
    stop()?;
    Ok(status())
}
#[tauri::command]
pub async fn diagnose_proxyswitch() -> Diagnostic {
    let s = status();
    if !s.tunnel_running {
        return Diagnostic {
            github_reachable: false,
            latency_ms: 0,
            git_proxy_configured: false,
            git_proxy_matches_tunnel: false,
            error: Some("SSH SOCKS 隧道未运行。".into()),
        };
    }
    let started = Instant::now();
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("socks5://{HOST}:{PORT}")).unwrap())
        .user_agent("ProxySwitch/1.0")
        .timeout(Duration::from_secs(15))
        .build();
    let outcome = match client {
        Ok(c) => c
            .get("https://api.github.com/rate_limit")
            .send()
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    let proxies: [String; 2] = ["http.proxy", "https.proxy"].map(|k| {
        Command::new("git")
            .args(["config", "--global", "--get", k])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default()
    });
    let configured = proxies.iter().any(|v| !v.is_empty());
    let matches = proxies.iter().any(|v| v.contains("127.0.0.1:7890"));
    Diagnostic {
        github_reachable: outcome.is_ok(),
        latency_ms: started.elapsed().as_millis() as u64,
        git_proxy_configured: configured,
        git_proxy_matches_tunnel: matches,
        error: outcome.err(),
    }
}
pub fn auto_connect() {
    let Ok(_operation) = TUNNEL_OPERATION.lock() else {
        log::warn!("自动连接失败: 代理操作锁异常。");
        return;
    };

    if !read().auto_connect || MANUAL_STOP_REQUESTED.load(Ordering::SeqCst) {
        return;
    }

    if let Err(e) = start() {
        log::warn!("自动连接失败: {e}");
        return;
    }

    // A click on "关闭代理" can arrive while the SSH process is starting.
    if MANUAL_STOP_REQUESTED.load(Ordering::SeqCst) {
        if let Err(e) = stop() {
            log::warn!("停止已取消的自动连接失败: {e}");
        }
        return;
    }

    let mut p = read();
    p.proxy_enabled = true;
    if let Err(e) = save(&p) {
        log::warn!("保存自动连接状态失败: {e}");
    }
}
