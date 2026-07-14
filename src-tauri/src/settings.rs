use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

pub const DEFAULT_UPDATE_CHECK_INTERVAL_MINUTES: u64 = 5;
const MAX_UPDATE_CHECK_INTERVAL_MINUTES: u64 = 24 * 60;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub default_scan_root: Option<String>,
    pub update_check_interval_minutes: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_scan_root: None,
            update_check_interval_minutes: DEFAULT_UPDATE_CHECK_INTERVAL_MINUTES,
        }
    }
}

pub fn load(path: &Path) -> Result<AppSettings, String> {
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let payload =
        fs::read(path).map_err(|error| format!("Luna could not read its settings: {error}"))?;
    serde_json::from_slice(&payload)
        .map_err(|error| format!("Luna's settings are not valid JSON: {error}"))
}

pub fn update_default_scan_root(path: &Path, root: String) -> Result<AppSettings, String> {
    if root.trim().is_empty() {
        return Err("Choose a default scan location first.".to_string());
    }

    let mut settings = load(path)?;
    settings.default_scan_root = Some(root);
    save(path, &settings)?;
    Ok(settings)
}

pub fn update_check_interval(path: &Path, interval_minutes: u64) -> Result<AppSettings, String> {
    if !(1..=MAX_UPDATE_CHECK_INTERVAL_MINUTES).contains(&interval_minutes) {
        return Err("Choose an update interval from 1 minute to 24 hours.".to_string());
    }

    let mut settings = load(path)?;
    settings.update_check_interval_minutes = interval_minutes;
    save(path, &settings)?;
    Ok(settings)
}

fn save(path: &Path, settings: &AppSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Luna could not create its settings folder: {error}"))?;
    }
    let payload = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Luna could not encode its settings: {error}"))?;
    fs::write(path, payload).map_err(|error| format!("Luna could not save its settings: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn default_scan_root_survives_a_settings_reload() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "luna-clean-settings-{}-{unique}",
            std::process::id()
        ));
        let path = directory.join("settings.json");

        update_default_scan_root(&path, "C:\\".to_string())
            .expect("the default scan root should be saved");
        let reloaded = load(&path).expect("the settings should reload");

        assert_eq!(reloaded.default_scan_root.as_deref(), Some("C:\\"));
        assert_eq!(
            reloaded.update_check_interval_minutes,
            DEFAULT_UPDATE_CHECK_INTERVAL_MINUTES
        );
        fs::remove_dir_all(directory).expect("temporary settings should be removed");
    }

    #[test]
    fn update_check_interval_defaults_and_survives_a_settings_reload() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "luna-clean-update-settings-{}-{unique}",
            std::process::id()
        ));
        let path = directory.join("settings.json");

        fs::create_dir_all(&directory).expect("temporary settings folder should be created");
        fs::write(&path, r#"{"defaultScanRoot":"C:\\"}"#)
            .expect("legacy settings should be written");
        assert_eq!(
            load(&path)
                .expect("legacy settings should load")
                .update_check_interval_minutes,
            DEFAULT_UPDATE_CHECK_INTERVAL_MINUTES
        );

        update_check_interval(&path, 30).expect("the update interval should be saved");
        assert_eq!(
            load(&path)
                .expect("the settings should reload")
                .update_check_interval_minutes,
            30
        );
        assert!(update_check_interval(&path, 0).is_err());
        assert!(update_check_interval(&path, MAX_UPDATE_CHECK_INTERVAL_MINUTES + 1).is_err());

        fs::remove_dir_all(directory).expect("temporary settings should be removed");
    }
}
