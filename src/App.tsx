import { invoke } from "@tauri-apps/api/core";
import { Activity, CircleAlert, Power, RefreshCw, Wifi } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

type ProxyStatus = "off" | "starting" | "on" | "stopping" | "error";

interface UpstreamProxyStatus {
  enabled: boolean;
  proxyUrl: string | null;
}

interface ProxySwitchDiagnostic {
  githubReachable: boolean;
  latencyMs: number;
  gitProxyConfigured: boolean;
  gitProxyMatchesTunnel: boolean;
  error: string | null;
}

interface TunnelStatus {
  state: string;
  tunnelRunning: boolean;
  portListening: boolean;
  pid: number | null;
  localSocks: string;
  remoteSsh: string;
  lastError: string | null;
}

const PROXY_URL = "socks5://127.0.0.1:7890";
const LOCAL_SOCKS = "127.0.0.1:7890";

const statusText: Record<ProxyStatus, string> = {
  off: "已关闭",
  starting: "启动中",
  on: "已开启",
  stopping: "关闭中",
  error: "异常",
};

const statusDescription: Record<ProxyStatus, string> = {
  off: "SSH SOCKS 隧道和应用出站代理均未启用。",
  starting: "正在建立 SSH SOCKS 隧道并切换应用网络连接。",
  on: "SSH SOCKS 隧道已建立，应用出站连接正在通过代理。",
  stopping: "正在关闭应用代理并停止 SSH SOCKS 隧道。",
  error: "代理状态异常，请检查 SSH 认证、服务器连接和本地端口。",
};

function formatTime(date: Date) {
  return new Intl.DateTimeFormat("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(date);
}

function App() {
  const [status, setStatus] = useState<ProxyStatus>("off");
  const [autoStart, setAutoStart] = useState(false);
  const [autoConnect, setAutoConnect] = useState(false);
  const [lastCheck, setLastCheck] = useState("尚未检测");
  const [testMessage, setTestMessage] = useState("尚未执行连通性检测");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isTesting, setIsTesting] = useState(false);
  const [isLocalProxyAvailable, setIsLocalProxyAvailable] = useState(false);
  const [isConfigured, setIsConfigured] = useState(false);
  const [isTunnelRunning, setIsTunnelRunning] = useState(false);
  const refreshInFlight = useRef(false);

  const isBusy = status === "starting" || status === "stopping";

  const statusClassName = useMemo(() => {
    if (status === "on") return "status-pill status-on";
    if (status === "error") return "status-pill status-error";
    if (isBusy) return "status-pill status-busy";
    return "status-pill status-off";
  }, [isBusy, status]);

  const refreshStatus = useCallback(async (showError = false) => {
    if (refreshInFlight.current) return;
    refreshInFlight.current = true;

    try {
      const [proxy, autoLaunch, autoConnectEnabled, tunnel] = await Promise.all(
        [
          invoke<UpstreamProxyStatus>("get_upstream_proxy_status"),
          invoke<boolean>("get_auto_launch_status"),
          invoke<boolean>("get_proxyswitch_auto_connect"),
          invoke<TunnelStatus>("get_proxyswitch_status"),
        ],
      );
      const configuredForLocalSocks = proxy.proxyUrl === PROXY_URL;

      setIsLocalProxyAvailable(tunnel.portListening);
      setIsTunnelRunning(tunnel.tunnelRunning);
      setIsConfigured(configuredForLocalSocks);
      setStatus(
        configuredForLocalSocks
          ? tunnel.tunnelRunning
            ? "on"
            : "error"
          : proxy.enabled || tunnel.state === "error" || tunnel.tunnelRunning
            ? "error"
            : "off",
      );
      setAutoStart(autoLaunch);
      setAutoConnect(autoConnectEnabled);
      if (showError) setErrorMessage(tunnel.lastError);
    } catch (error) {
      setStatus("error");
      if (showError) setErrorMessage(String(error));
    } finally {
      setIsLoading(false);
      refreshInFlight.current = false;
    }
  }, []);

  useEffect(() => {
    void refreshStatus(true);
    const interval = window.setInterval(() => void refreshStatus(false), 5_000);
    return () => window.clearInterval(interval);
  }, [refreshStatus]);

  const handleToggle = async () => {
    if (isBusy || isLoading) return;

    const nextEnabled = !(isConfigured || isTunnelRunning);
    setStatus(nextEnabled ? "starting" : "stopping");
    setErrorMessage(null);

    try {
      if (nextEnabled) {
        const tunnel = await invoke<TunnelStatus>("get_proxyswitch_status");
        let startedHere = false;
        if (!tunnel.tunnelRunning) {
          await invoke<TunnelStatus>("start_proxyswitch_tunnel");
          startedHere = true;
        }

        try {
          await invoke("set_global_proxy_url", { url: PROXY_URL });
        } catch (error) {
          if (startedHere) await invoke("stop_proxyswitch_tunnel");
          throw error;
        }
        setIsConfigured(true);
        setIsTunnelRunning(true);
        setIsLocalProxyAvailable(true);
        setStatus("on");
      } else {
        await invoke("set_global_proxy_url", { url: "" });
        await invoke<TunnelStatus>("stop_proxyswitch_tunnel");
        setIsConfigured(false);
        setIsTunnelRunning(false);
        setIsLocalProxyAvailable(false);
        setStatus("off");
      }
      setLastCheck(formatTime(new Date()));
    } catch (error) {
      setStatus("error");
      setErrorMessage(String(error));
    }
  };

  const handleDiagnosis = async () => {
    setIsTesting(true);
    setErrorMessage(null);

    try {
      const result = await invoke<ProxySwitchDiagnostic>(
        "diagnose_proxyswitch",
      );
      setLastCheck(formatTime(new Date()));
      if (result.githubReachable) {
        const gitStatus = result.gitProxyMatchesTunnel
          ? "Git 已指向本地代理"
          : result.gitProxyConfigured
            ? "Git 配置了其他代理"
            : "Git 未配置全局代理";
        setTestMessage(
          `GitHub 可达，检测耗时 ${result.latencyMs} ms；${gitStatus}`,
        );
      } else {
        setTestMessage("GitHub 连通性检测失败");
        setErrorMessage(result.error ?? "GitHub 不可达");
      }
    } catch (error) {
      setTestMessage("GitHub 连通性检测失败");
      setErrorMessage(String(error));
    } finally {
      setIsTesting(false);
    }
  };

  const handleAutoStartChange = async (enabled: boolean) => {
    const previous = autoStart;
    setAutoStart(enabled);
    setErrorMessage(null);

    try {
      await invoke("set_auto_launch", { enabled });
    } catch (error) {
      setAutoStart(previous);
      setErrorMessage(String(error));
    }
  };

  const handleAutoConnectChange = async (enabled: boolean) => {
    const previous = autoConnect;
    setAutoConnect(enabled);
    setErrorMessage(null);

    try {
      await invoke("set_proxyswitch_auto_connect", { enabled });
    } catch (error) {
      setAutoConnect(previous);
      setErrorMessage(String(error));
    }
  };

  return (
    <main className="app-shell">
      <section className="hero-card" aria-busy={isLoading}>
        <div className="hero-header">
          <div>
            <p className="eyebrow">ProxySwitch</p>
            <h1>代理开关</h1>
            <p className="subtitle">管理 SSH SOCKS 隧道及应用出站代理。</p>
          </div>
          <span className={statusClassName}>
            {isLoading ? "读取中" : statusText[status]}
          </span>
        </div>

        <div className="status-panel">
          <div>
            <p className="label">当前状态</p>
            <h2>{isLoading ? "正在读取配置" : statusText[status]}</h2>
            <p>
              {isLoading
                ? "正在同步代理与自启动设置。"
                : statusDescription[status]}
            </p>
          </div>
          <button
            className="primary-button"
            disabled={isBusy || isLoading}
            onClick={() => void handleToggle()}
          >
            <Power aria-hidden="true" size={18} />
            {isBusy
              ? statusText[status]
              : isConfigured || isTunnelRunning
                ? "关闭代理"
                : "开启代理"}
          </button>
        </div>

        <div className="grid-panel">
          <div className="info-card">
            <p className="label">本地 SOCKS</p>
            <strong>{LOCAL_SOCKS}</strong>
          </div>
          <div className="info-card">
            <p className="label">本地服务</p>
            <strong>{isLocalProxyAvailable ? "正在监听" : "未检测到"}</strong>
          </div>
          <div className="info-card">
            <p className="label">最近检测</p>
            <strong>{lastCheck}</strong>
          </div>
        </div>

        <div className="toolbar">
          <button
            className="secondary-button"
            disabled={isTesting}
            onClick={() => void handleDiagnosis()}
          >
            {isTesting ? (
              <RefreshCw className="spin" aria-hidden="true" size={18} />
            ) : (
              <Wifi aria-hidden="true" size={18} />
            )}
            {isTesting ? "检测中" : "检测 GitHub 连通性"}
          </button>
          <div className="startup-options">
            <label className="switch-row">
              <input
                checked={autoStart}
                disabled={isLoading}
                onChange={(event) =>
                  void handleAutoStartChange(event.target.checked)
                }
                type="checkbox"
              />
              <span>开机启动应用</span>
            </label>
            <label className="switch-row">
              <input
                checked={autoConnect}
                disabled={isLoading}
                onChange={(event) =>
                  void handleAutoConnectChange(event.target.checked)
                }
                type="checkbox"
              />
              <span>应用启动后自动连接代理</span>
            </label>
          </div>
        </div>

        <div className="diagnostic-result" aria-live="polite">
          <Activity aria-hidden="true" size={17} />
          <span>{testMessage}</span>
        </div>
        {errorMessage && (
          <div className="error-message" role="alert">
            <CircleAlert aria-hidden="true" size={18} />
            <span>{errorMessage}</span>
          </div>
        )}
      </section>
    </main>
  );
}

export default App;
