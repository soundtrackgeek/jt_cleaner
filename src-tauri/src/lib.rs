mod models;
mod scanner;

use models::{CleanupRequest, CleanupResult, ScanProgress, ScanResult, ScanRootInfo};
use serde::Serialize;
use tauri::Emitter;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    name: &'static str,
    version: &'static str,
    backend_ready: bool,
    default_scan_root: Option<String>,
}

#[tauri::command]
fn app_status() -> AppStatus {
    AppStatus {
        name: "Luna Clean",
        version: env!("CARGO_PKG_VERSION"),
        backend_ready: true,
        default_scan_root: scanner::default_scan_root().ok(),
    }
}

#[tauri::command]
fn list_scan_roots() -> Vec<ScanRootInfo> {
    scanner::list_scan_roots()
}

#[tauri::command]
async fn scan_path(app: tauri::AppHandle, path: String) -> Result<ScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        scanner::scan_path(&path, |progress: ScanProgress| {
            let _ = app.emit("scan-progress", progress);
        })
    })
    .await
    .map_err(|error| format!("The scan worker stopped unexpectedly: {error}"))?
}

#[tauri::command]
async fn clean_items(request: CleanupRequest) -> Result<CleanupResult, String> {
    tauri::async_runtime::spawn_blocking(move || scanner::clean_items(&request.item_ids))
        .await
        .map_err(|error| format!("The cleanup worker stopped unexpectedly: {error}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = dotenvy::dotenv();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            app_status,
            list_scan_roots,
            scan_path,
            clean_items
        ])
        .run(tauri::generate_context!())
        .expect("error while running Luna Clean");
}
