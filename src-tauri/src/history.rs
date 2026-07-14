use crate::models::{AgeBuckets, ScanResult};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

const SCHEMA_VERSION: u8 = 1;
const MAX_SNAPSHOTS_PER_ROOT: usize = 104;
const MAX_CATEGORIES_PER_SNAPSHOT: usize = 24;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotCategory {
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub last_used_days: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupSignal {
    pub id: String,
    pub size_bytes: u64,
    pub file_count: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSnapshot {
    pub captured_at: String,
    pub total_bytes: u64,
    pub file_count: u64,
    pub folder_count: u64,
    pub categories: Vec<SnapshotCategory>,
    pub age_buckets: AgeBuckets,
    pub cleanup_signals: Vec<CleanupSignal>,
    pub duplicate_reclaimable_bytes: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendHistory {
    pub root_id: String,
    pub root_name: String,
    pub snapshots: Vec<StorageSnapshot>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotStore {
    schema_version: u8,
    roots: HashMap<String, TrendHistory>,
}

pub fn history_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("trends").join("snapshots.json")
}

pub fn save_snapshot(path: &Path, result: &ScanResult) -> Result<TrendHistory, String> {
    let mut store = read_store(path)?;
    let root_id = root_id(&result.root);
    migrate_legacy_history(&mut store, &result.root, &root_id);
    let snapshot = snapshot_from_scan(result);
    let history = store
        .roots
        .entry(root_id.clone())
        .or_insert_with(|| TrendHistory {
            root_id,
            root_name: result.root_name.clone(),
            snapshots: Vec::new(),
        });

    history.root_name.clone_from(&result.root_name);
    insert_snapshot(&mut history.snapshots, snapshot);
    let updated = history.clone();
    write_store(path, &store)?;
    Ok(updated)
}

pub fn load_history(path: &Path, root: &str) -> Result<TrendHistory, String> {
    let store = read_store(path)?;
    let id = root_id(root);
    let mut history = store
        .roots
        .get(&id)
        .cloned()
        .or_else(|| {
            legacy_root_ids(root)
                .into_iter()
                .find_map(|legacy_id| store.roots.get(&legacy_id).cloned())
        })
        .unwrap_or_else(|| TrendHistory {
            root_id: id.clone(),
            root_name: display_name(root),
            snapshots: Vec::new(),
        });
    history.root_id = id;
    Ok(history)
}

pub fn clear_history(path: &Path, root: &str) -> Result<TrendHistory, String> {
    let mut store = read_store(path)?;
    let id = root_id(root);
    store.roots.remove(&id);
    for legacy_id in legacy_root_ids(root) {
        store.roots.remove(&legacy_id);
    }
    write_store(path, &store)?;
    Ok(TrendHistory {
        root_id: id,
        root_name: display_name(root),
        snapshots: Vec::new(),
    })
}

pub fn delete_snapshot(path: &Path, root: &str, captured_at: &str) -> Result<TrendHistory, String> {
    if captured_at.trim().is_empty() {
        return Err("Choose a snapshot to delete.".to_string());
    }

    let mut store = read_store(path)?;
    let id = root_id(root);
    migrate_legacy_history(&mut store, root, &id);
    let history = store
        .roots
        .get_mut(&id)
        .ok_or_else(|| "That snapshot no longer exists.".to_string())?;

    if !remove_snapshot(&mut history.snapshots, captured_at) {
        return Err("That snapshot no longer exists.".to_string());
    }

    let updated = history.clone();
    write_store(path, &store)?;
    Ok(updated)
}

fn snapshot_from_scan(result: &ScanResult) -> StorageSnapshot {
    let categories = result
        .categories
        .iter()
        .take(MAX_CATEGORIES_PER_SNAPSHOT)
        .map(|category| SnapshotCategory {
            id: root_id(&category.path),
            name: category.name.clone(),
            size_bytes: category.size_bytes,
            file_count: category.file_count,
            last_used_days: category.last_used_days,
        })
        .collect();
    let cleanup_signals = result
        .cleanup_items
        .iter()
        .map(|item| CleanupSignal {
            id: item.id.clone(),
            size_bytes: item.size_bytes,
            file_count: item.file_count,
        })
        .collect();

    StorageSnapshot {
        captured_at: result.scanned_at.clone(),
        total_bytes: result.reported_used_bytes(),
        file_count: result.file_count,
        folder_count: result.folder_count,
        categories,
        age_buckets: result.age_buckets.clone(),
        cleanup_signals,
        duplicate_reclaimable_bytes: result
            .duplicate_groups
            .iter()
            .map(|group| group.reclaimable_bytes)
            .sum(),
    }
}

fn insert_snapshot(snapshots: &mut Vec<StorageSnapshot>, snapshot: StorageSnapshot) {
    let day = snapshot
        .captured_at
        .get(..10)
        .unwrap_or(&snapshot.captured_at);
    if let Some(existing) = snapshots
        .iter_mut()
        .find(|entry| entry.captured_at.get(..10).unwrap_or(&entry.captured_at) == day)
    {
        *existing = snapshot;
    } else {
        snapshots.push(snapshot);
    }

    snapshots.sort_by(|left, right| left.captured_at.cmp(&right.captured_at));
    if snapshots.len() > MAX_SNAPSHOTS_PER_ROOT {
        snapshots.drain(..snapshots.len() - MAX_SNAPSHOTS_PER_ROOT);
    }
}

fn remove_snapshot(snapshots: &mut Vec<StorageSnapshot>, captured_at: &str) -> bool {
    let previous_len = snapshots.len();
    snapshots.retain(|snapshot| snapshot.captured_at != captured_at);
    snapshots.len() != previous_len
}

fn root_id(root: &str) -> String {
    let normalized = normalized_root(root);
    blake3::hash(normalized.as_bytes()).to_hex()[..16].to_string()
}

fn normalized_root(root: &str) -> String {
    let normalized = root.replace('/', "\\").to_lowercase();
    let without_extended_prefix = if let Some(path) = normalized.strip_prefix(r"\\?\unc\") {
        format!(r"\\{path}")
    } else if let Some(path) = normalized.strip_prefix(r"\\?\") {
        path.to_string()
    } else {
        normalized
    };
    let without_trailing_separator = without_extended_prefix.trim_end_matches('\\');
    if without_trailing_separator.is_empty() {
        "\\".to_string()
    } else {
        without_trailing_separator.to_string()
    }
}

fn legacy_root_id(root: &str) -> String {
    let normalized = root.replace('/', "\\").to_lowercase();
    blake3::hash(normalized.as_bytes()).to_hex()[..16].to_string()
}

fn legacy_root_ids(root: &str) -> Vec<String> {
    let normalized = root.replace('/', "\\").to_lowercase();
    let mut path_variants = vec![normalized.clone()];
    if let Some(path) = normalized.strip_prefix(r"\\?\unc\") {
        path_variants.push(format!(r"\\{path}"));
    } else if let Some(path) = normalized.strip_prefix(r"\\?\") {
        path_variants.push(path.to_string());
    } else if let Some(path) = normalized.strip_prefix(r"\\") {
        path_variants.push(format!(r"\\?\unc\{path}"));
    } else if normalized.as_bytes().get(1) == Some(&b':') {
        path_variants.push(format!(r"\\?\{normalized}"));
    }

    let mut ids = Vec::new();
    for variant in path_variants {
        let trimmed = variant.trim_end_matches('\\');
        for legacy_path in [variant.as_str(), trimmed] {
            if legacy_path.is_empty() {
                continue;
            }
            let id = legacy_root_id(legacy_path);
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

fn migrate_legacy_history(store: &mut SnapshotStore, root: &str, root_id: &str) {
    let mut legacy_snapshots = Vec::new();
    for legacy_id in legacy_root_ids(root) {
        if legacy_id != root_id {
            if let Some(history) = store.roots.remove(&legacy_id) {
                legacy_snapshots.extend(history.snapshots);
            }
        }
    }
    if legacy_snapshots.is_empty() {
        return;
    }

    let history = store
        .roots
        .entry(root_id.to_string())
        .or_insert_with(|| TrendHistory {
            root_id: root_id.to_string(),
            root_name: display_name(root),
            snapshots: Vec::new(),
        });
    for snapshot in legacy_snapshots {
        insert_snapshot(&mut history.snapshots, snapshot);
    }
}

fn display_name(root: &str) -> String {
    Path::new(root)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(root)
        .to_string()
}

fn read_store(path: &Path) -> Result<SnapshotStore, String> {
    if !path.exists() {
        return Ok(SnapshotStore {
            schema_version: SCHEMA_VERSION,
            roots: HashMap::new(),
        });
    }
    let bytes =
        fs::read(path).map_err(|error| format!("Luna could not read storage history: {error}"))?;
    let mut store: SnapshotStore = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Storage history is not valid JSON: {error}"))?;
    if store.schema_version == 0 {
        store.schema_version = SCHEMA_VERSION;
    }
    Ok(store)
}

fn write_store(path: &Path, store: &SnapshotStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Luna could not create the trends folder: {error}"))?;
    }
    let mut payload = serde_json::to_vec(store)
        .map_err(|error| format!("Luna could not encode storage history: {error}"))?;
    payload.push(b'\n');
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, payload)
        .map_err(|error| format!("Luna could not write storage history: {error}"))?;
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("Luna could not replace storage history: {error}"))?;
    }
    fs::rename(&temporary, path)
        .map_err(|error| format!("Luna could not finish storage history: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Local};

    fn sample_snapshot(day: usize, size: u64) -> StorageSnapshot {
        StorageSnapshot {
            captured_at: format!("2026-01-{:02}T12:00:00+00:00", day),
            total_bytes: size,
            file_count: 1,
            folder_count: 1,
            categories: Vec::new(),
            age_buckets: AgeBuckets::default(),
            cleanup_signals: Vec::new(),
            duplicate_reclaimable_bytes: 0,
        }
    }

    #[test]
    fn replaces_a_second_snapshot_from_the_same_day() {
        let mut snapshots = vec![sample_snapshot(1, 10)];
        insert_snapshot(&mut snapshots, sample_snapshot(1, 20));
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].total_bytes, 20);
    }

    #[test]
    fn deletes_only_the_snapshot_with_the_exact_capture_time() {
        let mut snapshots = vec![sample_snapshot(1, 10), sample_snapshot(2, 20)];

        assert!(remove_snapshot(&mut snapshots, "2026-01-01T12:00:00+00:00"));
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].total_bytes, 20);
        assert!(!remove_snapshot(
            &mut snapshots,
            "2026-01-03T12:00:00+00:00"
        ));
    }

    #[test]
    fn treats_windows_extended_and_display_paths_as_the_same_root() {
        assert_eq!(root_id(r"C:\"), root_id(r"\\?\C:\"));
        assert_eq!(root_id(r"C:\Users\Luna\"), root_id(r"\\?\C:\Users\Luna"));
        assert_eq!(
            root_id(r"\\server\share\photos"),
            root_id(r"\\?\UNC\server\share\photos\")
        );
    }

    #[test]
    fn finds_and_migrates_snapshots_saved_with_the_legacy_windows_id() {
        let display_root = r"C:\";
        let extended_root = r"\\?\C:\";
        let current_id = root_id(display_root);
        let legacy_id = legacy_root_id(extended_root);
        assert_ne!(current_id, legacy_id);
        assert!(legacy_root_ids(display_root).contains(&legacy_id));

        let mut store = SnapshotStore {
            schema_version: SCHEMA_VERSION,
            roots: HashMap::from([(
                legacy_id.clone(),
                TrendHistory {
                    root_id: legacy_id.clone(),
                    root_name: "Local Disk (C:)".to_string(),
                    snapshots: vec![sample_snapshot(1, 42)],
                },
            )]),
        };
        migrate_legacy_history(&mut store, extended_root, &current_id);

        assert!(!store.roots.contains_key(&legacy_id));
        assert_eq!(store.roots[&current_id].snapshots[0].total_bytes, 42);
    }

    #[test]
    fn caps_history_at_two_years_of_weekly_entries() {
        let mut snapshots = Vec::new();
        for index in 0..110 {
            let date = DateTime::from_timestamp(1_700_000_000 + index * 604_800, 0)
                .unwrap()
                .with_timezone(&Local)
                .to_rfc3339();
            let mut snapshot = sample_snapshot(1, index as u64);
            snapshot.captured_at = date;
            insert_snapshot(&mut snapshots, snapshot);
        }
        assert_eq!(snapshots.len(), MAX_SNAPSHOTS_PER_ROOT);
        assert_eq!(snapshots.last().unwrap().total_bytes, 109);
    }
}
