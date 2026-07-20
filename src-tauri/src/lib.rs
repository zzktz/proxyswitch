mod auto_launch;
mod commands;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }
            tauri::async_runtime::spawn_blocking(commands::auto_connect);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_proxyswitch_status,
            commands::enable_proxyswitch,
            commands::disable_proxyswitch,
            commands::diagnose_proxyswitch,
            commands::set_auto_launch,
            commands::get_auto_launch_status,
            commands::set_proxyswitch_auto_connect
        ])
        .run(tauri::generate_context!())
        .expect("failed to run ProxySwitch");
}
