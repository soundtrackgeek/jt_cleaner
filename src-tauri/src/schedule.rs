use chrono::{DateTime, Duration, Local, SecondsFormat};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSettings {
    pub enabled: bool,
    pub frequency: String,
    pub scan_root: Option<String>,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleUpdate {
    pub enabled: bool,
    pub frequency: String,
    pub scan_root: String,
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            frequency: "Weekly".to_string(),
            scan_root: None,
            last_run_at: None,
            next_run_at: None,
            last_error: None,
        }
    }
}

pub fn load(path: &Path) -> Result<ScheduleSettings, String> {
    if !path.exists() {
        return Ok(ScheduleSettings::default());
    }
    let payload = fs::read(path)
        .map_err(|error| format!("Luna could not read schedule settings: {error}"))?;
    serde_json::from_slice(&payload)
        .map_err(|error| format!("Schedule settings are not valid JSON: {error}"))
}

pub fn update(path: &Path, request: ScheduleUpdate) -> Result<ScheduleSettings, String> {
    if !matches!(request.frequency.as_str(), "Daily" | "Weekly" | "Monthly") {
        return Err("Choose Daily, Weekly, or Monthly scheduling.".to_string());
    }
    if request.scan_root.trim().is_empty() {
        return Err("Choose a scan location before enabling a schedule.".to_string());
    }

    let previous = load(path)?;
    let schedule_changed = previous.frequency != request.frequency
        || previous.scan_root.as_deref() != Some(request.scan_root.as_str())
        || previous.enabled != request.enabled;
    let next_run_at = if request.enabled {
        if schedule_changed || previous.next_run_at.is_none() {
            Some(next_after(Local::now(), &request.frequency))
        } else {
            previous.next_run_at
        }
    } else {
        None
    };
    let settings = ScheduleSettings {
        enabled: request.enabled,
        frequency: request.frequency,
        scan_root: Some(request.scan_root),
        last_run_at: previous.last_run_at,
        next_run_at,
        last_error: None,
    };
    save(path, &settings)?;
    Ok(settings)
}

pub fn is_due(settings: &ScheduleSettings) -> bool {
    if !settings.enabled {
        return false;
    }
    settings
        .next_run_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .is_none_or(|next| next <= Local::now())
}

pub fn mark_capture(path: &Path, root: &str) -> Result<ScheduleSettings, String> {
    let mut settings = load(path)?;
    if settings.enabled && settings.scan_root.as_deref() == Some(root) {
        let now = Local::now();
        settings.last_run_at = Some(now.to_rfc3339_opts(SecondsFormat::Secs, true));
        settings.next_run_at = Some(next_after(now, &settings.frequency));
        settings.last_error = None;
        save(path, &settings)?;
    }
    Ok(settings)
}

pub fn mark_error(path: &Path, message: &str) -> Result<ScheduleSettings, String> {
    let mut settings = load(path)?;
    settings.last_error = Some(message.to_string());
    settings.next_run_at =
        Some((Local::now() + Duration::hours(6)).to_rfc3339_opts(SecondsFormat::Secs, true));
    save(path, &settings)?;
    Ok(settings)
}

fn next_after(now: DateTime<Local>, frequency: &str) -> String {
    let duration = match frequency {
        "Daily" => Duration::days(1),
        "Monthly" => Duration::days(30),
        _ => Duration::weeks(1),
    };
    (now + duration).to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn save(path: &Path, settings: &ScheduleSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Luna could not create its settings folder: {error}"))?;
    }
    let payload = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Luna could not encode schedule settings: {error}"))?;
    fs::write(path, payload)
        .map_err(|error| format!("Luna could not write schedule settings: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_schedules_are_never_due() {
        assert!(!is_due(&ScheduleSettings::default()));
    }

    #[test]
    fn an_enabled_schedule_without_a_date_is_due() {
        let settings = ScheduleSettings {
            enabled: true,
            scan_root: Some("C:\\".to_string()),
            ..ScheduleSettings::default()
        };
        assert!(is_due(&settings));
    }
}
