import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import appPackage from "../package.json";

interface Status { state: "on" | "off" | "error"; tunnelRunning: boolean; portListening: boolean; proxyEnabled: boolean; autoConnect: boolean; lastError: string | null; }
interface Diagnostic { githubReachable: boolean; latencyMs: number; gitProxyConfigured: boolean; gitProxyMatchesTunnel: boolean; error: string | null; }

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [autoLaunch, setAutoLaunch] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("尚未检测 GitHub 连通性");
  const refresh = useCallback(async () => { const [next, launch] = await Promise.all([invoke<Status>("get_proxyswitch_status"), invoke<boolean>("get_auto_launch_status")]); setStatus(next); setAutoLaunch(launch); }, []);
  useEffect(() => { void refresh(); const timer = window.setInterval(() => void refresh(), 5000); return () => window.clearInterval(timer); }, [refresh]);
  const enable = async () => { if (!status) return; setBusy(true); try { await invoke("enable_proxyswitch"); await refresh(); } catch (error) { setMessage(String(error)); } finally { setBusy(false); } };
  const disable = async () => { setBusy(true); try { await invoke("disable_proxyswitch"); await refresh(); } catch (error) { setMessage(String(error)); } finally { setBusy(false); } };
  const diagnose = async () => { setBusy(true); try { const result = await invoke<Diagnostic>("diagnose_proxyswitch"); setMessage(result.githubReachable ? `GitHub 可达，${result.latencyMs} ms；${result.gitProxyMatchesTunnel ? "Git 已指向本地代理" : result.gitProxyConfigured ? "Git 使用其他代理" : "Git 未配置全局代理"}` : result.error ?? "GitHub 不可达"); } finally { setBusy(false); } };
  const setStartup = async (command: string, enabled: boolean) => { setBusy(true); try { await invoke(command, { enabled }); await refresh(); } catch (error) { setMessage(String(error)); } finally { setBusy(false); } };
  const on = status?.state === "on";
  const stateLabel = !status ? "读取中" : on ? "代理已开启" : status.state === "error" ? "代理异常" : "代理已关闭";
  const stateTone = !status ? "pending" : on ? "running" : status.state === "error" ? "error" : "stopped";
  return <main><section><header><div><p>ProxySwitch</p><h1>SSH SOCKS 代理</h1><span className={`overall-status ${stateTone}`}><i />{stateLabel}</span></div>{on || status?.proxyEnabled || status?.tunnelRunning ? <button className="danger" disabled={busy} onClick={() => void disable()}>关闭代理</button> : <button disabled={!status || busy} onClick={() => void enable()}>开启代理</button>}</header><dl><div><dt>本地 SOCKS</dt><dd>127.0.0.1:7890</dd></div><div><dt>SSH 隧道</dt><dd className={status?.tunnelRunning ? "state running" : "state stopped"}><i />{status?.tunnelRunning ? "运行中" : "未运行"}</dd></div><div><dt>本地端口</dt><dd className={status?.portListening ? "state running" : "state stopped"}><i />{status?.portListening ? "正在监听" : "未监听"}</dd></div></dl><div className="actions"><button disabled={!status || busy} onClick={() => void diagnose()}>检测 GitHub</button><label><input checked={autoLaunch} disabled={busy} onChange={(e) => void setStartup("set_auto_launch", e.target.checked)} type="checkbox" />开机启动应用</label><label><input checked={status?.autoConnect ?? false} disabled={busy} onChange={(e) => void setStartup("set_proxyswitch_auto_connect", e.target.checked)} type="checkbox" />启动后自动连接</label></div><p className={`message ${status?.lastError ? "error" : ""}`}>{status?.lastError ?? message}</p></section><small className="app-version">v{appPackage.version}</small></main>;
}
