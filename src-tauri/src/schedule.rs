use chrono::{DateTime, Duration, Local, Months, NaiveDate, NaiveTime, SecondsFormat, TimeZone};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

const DEFAULT_RUN_TIME: &str = "09:00";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSettings {
    pub enabled: bool,
    pub frequency: String,
    #[serde(default = "default_run_time")]
    pub run_time: String,
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
    pub run_time: String,
    pub scan_root: String,
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            frequency: "Weekly".to_string(),
            run_time: default_run_time(),
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
    let value: serde_json::Value = serde_json::from_slice(&payload)
        .map_err(|error| format!("Schedule settings are not valid JSON: {error}"))?;
    let needs_time_migration = value.get("runTime").is_none();
    let mut settings: ScheduleSettings = serde_json::from_value(value)
        .map_err(|error| format!("Schedule settings are not valid JSON: {error}"))?;
    if needs_time_migration {
        if settings.enabled {
            settings.next_run_at = Some(next_after(
                Local::now(),
                &settings.frequency,
                &settings.run_time,
            )?);
        }
        save(path, &settings)?;
    }
    Ok(settings)
}

pub fn update(path: &Path, request: ScheduleUpdate) -> Result<ScheduleSettings, String> {
    if !matches!(request.frequency.as_str(), "Daily" | "Weekly" | "Monthly") {
        return Err("Choose Daily, Weekly, or Monthly scheduling.".to_string());
    }
    let run_time = parse_run_time(&request.run_time)?
        .format("%H:%M")
        .to_string();
    if request.scan_root.trim().is_empty() {
        return Err("Choose a scan location before enabling a schedule.".to_string());
    }

    let previous = load(path)?;
    let schedule_changed = previous.frequency != request.frequency
        || previous.run_time != run_time
        || previous.scan_root.as_deref() != Some(request.scan_root.as_str())
        || previous.enabled != request.enabled;
    let next_run_at = if request.enabled {
        if schedule_changed || previous.next_run_at.is_none() {
            Some(next_after(Local::now(), &request.frequency, &run_time)?)
        } else {
            previous.next_run_at.clone()
        }
    } else {
        None
    };
    let settings = ScheduleSettings {
        enabled: request.enabled,
        frequency: request.frequency,
        run_time,
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
        settings.next_run_at = Some(next_after(now, &settings.frequency, &settings.run_time)?);
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

fn default_run_time() -> String {
    DEFAULT_RUN_TIME.to_string()
}

fn parse_run_time(value: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(value.trim(), "%H:%M")
        .map_err(|_| "Choose a valid capture time in 24-hour HH:MM format.".to_string())
}

fn next_after(now: DateTime<Local>, frequency: &str, run_time: &str) -> Result<String, String> {
    let time = parse_run_time(run_time)?;
    let today = local_at(now.date_naive(), time)?;
    let next = if today > now {
        today
    } else {
        let next_date = match frequency {
            "Daily" => now.date_naive().succ_opt(),
            "Monthly" => now.date_naive().checked_add_months(Months::new(1)),
            _ => now.date_naive().checked_add_signed(Duration::weeks(1)),
        }
        .ok_or_else(|| "Luna could not calculate the next scheduled date.".to_string())?;
        local_at(next_date, time)?
    };
    Ok(next.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn local_at(date: NaiveDate, time: NaiveTime) -> Result<DateTime<Local>, String> {
    let requested = date.and_time(time);
    (0..=180)
        .find_map(|minutes| {
            Local
                .from_local_datetime(&(requested + Duration::minutes(minutes)))
                .earliest()
        })
        .ok_or_else(|| {
            "Luna could not resolve the selected time in the local time zone.".to_string()
        })
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
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn legacy_schedules_default_to_nine_in_the_morning() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "luna-clean-schedule-{}-{unique}",
            std::process::id()
        ));
        let path = directory.join("schedule.json");
        fs::create_dir_all(&directory).expect("the temporary folder should be created");
        fs::write(
            &path,
            r#"{
                "enabled": true,
                "frequency": "Daily",
                "scanRoot": "C:\\",
                "lastRunAt": null,
                "nextRunAt": "2020-01-01T12:34:00Z",
                "lastError": null
            }"#,
        )
        .expect("the legacy schedule should be written");

        let settings = load(&path).expect("legacy schedules should still load");
        let persisted = fs::read_to_string(&path).expect("the migrated schedule should be saved");

        assert_eq!(settings.run_time, DEFAULT_RUN_TIME);
        assert!(settings.next_run_at.as_deref() != Some("2020-01-01T12:34:00Z"));
        assert!(persisted.contains(r#""runTime": "09:00""#));

        fs::remove_dir_all(directory).expect("the temporary folder should be removed");
    }

    #[test]
    fn daily_schedules_use_the_selected_local_time() {
        let before = Local
            .with_ymd_and_hms(2026, 7, 14, 7, 45, 0)
            .single()
            .expect("the test time should be valid");
        let after = Local
            .with_ymd_and_hms(2026, 7, 14, 8, 45, 0)
            .single()
            .expect("the test time should be valid");

        let same_day = DateTime::parse_from_rfc3339(
            &next_after(before, "Daily", "08:30").expect("the schedule should resolve"),
        )
        .expect("the next run should be an RFC 3339 timestamp");
        let next_day = DateTime::parse_from_rfc3339(
            &next_after(after, "Daily", "08:30").expect("the schedule should resolve"),
        )
        .expect("the next run should be an RFC 3339 timestamp");

        assert_eq!(
            same_day.format("%Y-%m-%d %H:%M").to_string(),
            "2026-07-14 08:30"
        );
        assert_eq!(
            next_day.format("%Y-%m-%d %H:%M").to_string(),
            "2026-07-15 08:30"
        );
    }

    #[test]
    fn invalid_capture_times_are_rejected() {
        let now = Local::now();
        assert!(next_after(now, "Daily", "25:00").is_err());
    }
}
