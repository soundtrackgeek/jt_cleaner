mod ai;
mod history;
mod models;
mod scanner;
mod schedule;
mod settings;

use models::{
    CleanupRequest, CleanupResult, ScanProgress, ScanResult, ScanRootInfo, StorageCategory,
};
use serde::Serialize;
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};
use tauri::{
    AppHandle, Emitter, Manager, State, WebviewWindowBuilder, WindowEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

#[derive(Default)]
struct RuntimeState {
    scan_running: Arc<AtomicBool>,
    storage_index: Arc<RwLock<scanner::StorageIndex>>,
    quitting: AtomicBool,
}

struct ScanPermit(Arc<AtomicBool>);

impl Drop for ScanPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    name: &'static str,
    version: &'static str,
    backend_ready: bool,
    default_scan_root: Option<String>,
    update_check_interval_minutes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScheduleStatus {
    #[serde(flatten)]
    settings: schedule::ScheduleSettings,
    is_scanning: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScheduledScanEvent {
    root: String,
    total_bytes: u64,
    scanned_at: String,
}

#[tauri::command]
fn app_status(app: AppHandle) -> AppStatus {
    let app_settings = settings_file(&app)
        .ok()
        .and_then(|path| settings::load(&path).ok())
        .unwrap_or_default();
    let default_scan_root = app_settings
        .default_scan_root
        .or_else(|| scanner::default_scan_root().ok());
    AppStatus {
        name: "Luna Clean",
        version: env!("CARGO_PKG_VERSION"),
        backend_ready: true,
        default_scan_root,
        update_check_interval_minutes: app_settings.update_check_interval_minutes,
    }
}

#[tauri::command]
fn list_scan_roots() -> Vec<ScanRootInfo> {
    scanner::list_scan_roots()
}

#[tauri::command]
fn ai_status() -> ai::AiStatus {
    ai::status()
}

#[tauri::command]
async fn save_api_key(request: ai::SaveApiKeyRequest) -> Result<ai::AiStatus, String> {
    ai::save_api_key(request).await
}

#[tauri::command]
async fn delete_api_key() -> Result<ai::AiStatus, String> {
    ai::delete_api_key().await
}

#[tauri::command]
async fn generate_ai_report(request: ai::AiReportRequest) -> Result<ai::AiReportEnvelope, String> {
    ai::investigate(request).await
}

#[tauri::command]
async fn scan_path(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    path: String,
) -> Result<ScanResult, String> {
    let permit = acquire_scan(state.scan_running.clone())?;
    let storage_index = state.storage_index.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let output = perform_scan(&app, &path, true)?;
        *storage_index
            .write()
            .map_err(|_| "The storage explorer index is unavailable.".to_string())? =
            output.storage_index;
        Ok(output.result)
    })
    .await
    .map_err(|error| format!("The scan worker stopped unexpectedly: {error}"))?
}

#[tauri::command]
fn list_storage_areas(
    state: State<'_, RuntimeState>,
    path: String,
) -> Result<Vec<StorageCategory>, String> {
    state
        .storage_index
        .read()
        .map_err(|_| "The storage explorer index is unavailable.".to_string())?
        .areas_for(&path)
}

#[tauri::command]
fn get_trend_history(app: AppHandle, root: String) -> Result<history::TrendHistory, String> {
    history::load_history(&history_file(&app)?, &root)
}

#[tauri::command]
fn clear_trend_history(app: AppHandle, root: String) -> Result<history::TrendHistory, String> {
    history::clear_history(&history_file(&app)?, &root)
}

#[tauri::command]
fn delete_trend_snapshot(
    app: AppHandle,
    root: String,
    captured_at: String,
) -> Result<history::TrendHistory, String> {
    history::delete_snapshot(&history_file(&app)?, &root, &captured_at)
}

#[tauri::command]
fn get_schedule_status(
    app: AppHandle,
    state: State<'_, RuntimeState>,
) -> Result<ScheduleStatus, String> {
    Ok(ScheduleStatus {
        settings: schedule::load(&schedule_file(&app)?)?,
        is_scanning: state.scan_running.load(Ordering::Acquire),
    })
}

#[tauri::command]
fn update_schedule(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    request: schedule::ScheduleUpdate,
) -> Result<ScheduleStatus, String> {
    Ok(ScheduleStatus {
        settings: schedule::update(&schedule_file(&app)?, request)?,
        is_scanning: state.scan_running.load(Ordering::Acquire),
    })
}

#[tauri::command]
fn update_default_scan_root(app: AppHandle, root: String) -> Result<settings::AppSettings, String> {
    settings::update_default_scan_root(&settings_file(&app)?, root)
}

#[tauri::command]
fn update_update_check_interval(
    app: AppHandle,
    interval_minutes: u64,
) -> Result<settings::AppSettings, String> {
    settings::update_check_interval(&settings_file(&app)?, interval_minutes)
}

#[tauri::command]
fn capture_scheduled_snapshot(
    app: AppHandle,
    state: State<'_, RuntimeState>,
) -> Result<(), String> {
    if state.scan_running.load(Ordering::Acquire) {
        return Err("A scan is already running.".to_string());
    }
    queue_background_scan(app, state.scan_running.clone(), true);
    Ok(())
}

#[tauri::command]
async fn clean_items(request: CleanupRequest) -> Result<CleanupResult, String> {
    tauri::async_runtime::spawn_blocking(move || scanner::clean_items(&request.item_ids))
        .await
        .map_err(|error| format!("The cleanup worker stopped unexpectedly: {error}"))
}

fn acquire_scan(flag: Arc<AtomicBool>) -> Result<ScanPermit, String> {
    flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ScanPermit(flag))
        .map_err(|_| "A scan is already running.".to_string())
}

fn should_prevent_exit(quitting: bool, code: Option<i32>) -> bool {
    !quitting && code != Some(tauri::RESTART_EXIT_CODE)
}

fn remembered_window_state() -> StateFlags {
    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED
}

fn app_data_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|error| format!("Luna could not locate its data folder: {error}"))
}

fn history_file(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(history::history_path(&app_data_dir(app)?))
}

fn schedule_file(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app_data_dir(app)?.join("schedule.json"))
}

fn settings_file(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app_data_dir(app)?.join("settings.json"))
}

fn perform_scan(
    app: &AppHandle,
    path: &str,
    emit_progress: bool,
) -> Result<scanner::ScanOutput, String> {
    let output = scanner::scan_path(path, |progress: ScanProgress| {
        if emit_progress {
            let _ = app.emit("scan-progress", progress);
        }
    })?;
    history::save_snapshot(&history_file(app)?, &output.result)?;
    schedule::mark_capture(&schedule_file(app)?, path)?;
    Ok(output)
}

fn queue_background_scan(app: AppHandle, flag: Arc<AtomicBool>, force: bool) {
    tauri::async_runtime::spawn(async move {
        background_scan(app, flag, force).await;
    });
}

async fn background_scan(app: AppHandle, flag: Arc<AtomicBool>, force: bool) {
    let schedule_path = match schedule_file(&app) {
        Ok(path) => path,
        Err(_) => return,
    };
    let settings = match schedule::load(&schedule_path) {
        Ok(settings) => settings,
        Err(error) => {
            let _ = app.emit("scheduled-scan-error", error);
            return;
        }
    };
    if !force && !schedule::is_due(&settings) {
        return;
    }
    let root = settings
        .scan_root
        .or_else(|| scanner::default_scan_root().ok());
    let Some(root) = root else {
        return;
    };
    let permit = match acquire_scan(flag) {
        Ok(permit) => permit,
        Err(_) => return,
    };
    let _ = app.emit("scheduled-scan-started", &root);
    let worker_app = app.clone();
    let worker_root = root.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        perform_scan(&worker_app, &worker_root, false)
    })
    .await;

    match outcome {
        Ok(Ok(output)) => {
            let result = output.result;
            let total_bytes = result.reported_used_bytes();
            let _ = app.emit(
                "scheduled-scan-complete",
                ScheduledScanEvent {
                    root: result.root,
                    total_bytes,
                    scanned_at: result.scanned_at,
                },
            );
        }
        Ok(Err(error)) => {
            let _ = schedule::mark_error(&schedule_path, &error);
            let _ = app.emit("scheduled-scan-error", error);
        }
        Err(error) => {
            let message = format!("The scheduled scan worker stopped unexpectedly: {error}");
            let _ = schedule::mark_error(&schedule_path, &message);
            let _ = app.emit("scheduled-scan-error", message);
        }
    }
}

fn show_main_window(app: &AppHandle) -> Result<(), String> {
    let window = if let Some(window) = app.get_webview_window("main") {
        window
    } else {
        let config = app
            .config()
            .app
            .windows
            .iter()
            .find(|config| config.label == "main")
            .cloned()
            .ok_or_else(|| "The main window configuration is missing.".to_string())?;
        WebviewWindowBuilder::from_config(app, &config)
            .map_err(|error| format!("Luna could not prepare its window: {error}"))?
            .build()
            .map_err(|error| format!("Luna could not open its window: {error}"))?
    };
    window
        .show()
        .map_err(|error| format!("Luna could not show its window: {error}"))?;
    window
        .set_focus()
        .map_err(|error| format!("Luna could not focus its window: {error}"))
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Luna Clean", true, None::<&str>)?;
    let capture = MenuItem::with_id(
        app,
        "capture",
        "Capture storage snapshot",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Luna Clean", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &capture, &separator, &quit])?;
    let mut tray = TrayIconBuilder::with_id("luna-clean")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Luna Clean — storage watch")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                let _ = show_main_window(app);
            }
            "capture" => {
                let flag = app.state::<RuntimeState>().scan_running.clone();
                queue_background_scan(app.clone(), flag, true);
            }
            "quit" => {
                app.state::<RuntimeState>()
                    .quitting
                    .store(true, Ordering::Release);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                let _ = show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

fn start_schedule_monitor(app: AppHandle, flag: Arc<AtomicBool>) {
    tauri::async_runtime::spawn(async move {
        loop {
            background_scan(app.clone(), flag.clone(), false).await;
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = dotenvy::dotenv();
    let app = tauri::Builder::default()
        .manage(RuntimeState::default())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--hidden"])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(remembered_window_state())
                .build(),
        )
        .setup(|app| {
            setup_tray(app)?;
            let flag = app.state::<RuntimeState>().scan_running.clone();
            start_schedule_monitor(app.handle().clone(), flag);
            if !std::env::args().any(|argument| argument == "--hidden") {
                show_main_window(app.handle()).map_err(std::io::Error::other)?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window
                    .app_handle()
                    .save_window_state(remembered_window_state());
                let _ = window.destroy();
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_status,
            list_scan_roots,
            scan_path,
            list_storage_areas,
            clean_items,
            get_trend_history,
            clear_trend_history,
            delete_trend_snapshot,
            ai_status,
            save_api_key,
            delete_api_key,
            generate_ai_report,
            get_schedule_status,
            update_schedule,
            update_default_scan_root,
            update_update_check_interval,
            capture_scheduled_snapshot
        ])
        .build(tauri::generate_context!())
        .expect("error while building Luna Clean");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            if should_prevent_exit(
                app.state::<RuntimeState>().quitting.load(Ordering::Acquire),
                code,
            ) {
                api.prevent_exit();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_guard_allows_explicit_quit_and_updater_restart() {
        assert!(should_prevent_exit(false, None));
        assert!(!should_prevent_exit(true, None));
        assert!(!should_prevent_exit(false, Some(tauri::RESTART_EXIT_CODE)));
    }

    #[test]
    fn window_state_tracks_geometry_without_restoring_visibility() {
        let flags = remembered_window_state();
        assert!(flags.contains(StateFlags::SIZE));
        assert!(flags.contains(StateFlags::POSITION));
        assert!(flags.contains(StateFlags::MAXIMIZED));
        assert!(!flags.contains(StateFlags::VISIBLE));
    }
}
