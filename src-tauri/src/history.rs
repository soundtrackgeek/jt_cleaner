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
    Ok(store
        .roots
        .get(&id)
        .cloned()
        .unwrap_or_else(|| TrendHistory {
            root_id: id,
            root_name: display_name(root),
            snapshots: Vec::new(),
        }))
}

pub fn clear_history(path: &Path, root: &str) -> Result<TrendHistory, String> {
    let mut store = read_store(path)?;
    let id = root_id(root);
    store.roots.remove(&id);
    write_store(path, &store)?;
    Ok(TrendHistory {
        root_id: id,
        root_name: display_name(root),
        snapshots: Vec::new(),
    })
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

fn root_id(root: &str) -> String {
    let normalized = root.replace('/', "\\").to_lowercase();
    blake3::hash(normalized.as_bytes()).to_hex()[..16].to_string()
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
