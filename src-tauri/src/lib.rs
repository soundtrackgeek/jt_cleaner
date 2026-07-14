mod history;
mod models;
mod scanner;

use models::{CleanupRequest, CleanupResult, ScanProgress, ScanResult, ScanRootInfo};
use serde::Serialize;
use tauri::{Emitter, Manager};

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
        let result = scanner::scan_path(&path, |progress: ScanProgress| {
            let _ = app.emit("scan-progress", progress);
        })?;
        let history_path = history::history_path(
            &app.path()
                .app_data_dir()
                .map_err(|error| format!("Luna could not locate its data folder: {error}"))?,
        );
        history::save_snapshot(&history_path, &result)?;
        Ok(result)
    })
    .await
    .map_err(|error| format!("The scan worker stopped unexpectedly: {error}"))?
}

#[tauri::command]
fn get_trend_history(app: tauri::AppHandle, root: String) -> Result<history::TrendHistory, String> {
    let path = history::history_path(
        &app.path()
            .app_data_dir()
            .map_err(|error| format!("Luna could not locate its data folder: {error}"))?,
    );
    history::load_history(&path, &root)
}

#[tauri::command]
fn clear_trend_history(
    app: tauri::AppHandle,
    root: String,
) -> Result<history::TrendHistory, String> {
    let path = history::history_path(
        &app.path()
            .app_data_dir()
            .map_err(|error| format!("Luna could not locate its data folder: {error}"))?,
    );
    history::clear_history(&path, &root)
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
            clean_items,
            get_trend_history,
            clear_trend_history
        ])
        .run(tauri::generate_context!())
        .expect("error while running Luna Clean");
}
