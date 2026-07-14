use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    name: &'static str,
    version: &'static str,
    backend_ready: bool,
}

#[tauri::command]
fn app_status() -> AppStatus {
    AppStatus {
        name: "Luna Clean",
        version: env!("CARGO_PKG_VERSION"),
        backend_ready: true,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![app_status])
        .run(tauri::generate_context!())
        .expect("error while running Luna Clean");
}

