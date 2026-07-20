use crate::scanner::ScanOutput;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

const SCHEMA_VERSION: u8 = 1;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LatestScanStore {
    schema_version: u8,
    scan: ScanOutput,
}

pub fn save(path: &Path, scan: &ScanOutput) -> Result<(), String> {
    let store = LatestScanStore {
        schema_version: SCHEMA_VERSION,
        scan: ScanOutput {
            result: scan.result.clone(),
            storage_index: scan.storage_index.clone(),
        },
    };
    write_store(path, &store)
}

pub fn load(path: &Path) -> Result<Option<ScanOutput>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("Luna could not read the latest scan snapshot: {error}"))?;
    let store: LatestScanStore = serde_json::from_slice(&bytes)
        .map_err(|error| format!("The latest scan snapshot is not valid JSON: {error}"))?;
    if store.schema_version != SCHEMA_VERSION {
        return Ok(None);
    }
    Ok(Some(store.scan))
}

fn write_store(path: &Path, store: &LatestScanStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Luna could not create its scan snapshot folder: {error}"))?;
    }
    let mut payload = serde_json::to_vec(store)
        .map_err(|error| format!("Luna could not encode the latest scan snapshot: {error}"))?;
    payload.push(b'\n');
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, payload)
        .map_err(|error| format!("Luna could not write the latest scan snapshot: {error}"))?;
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("Luna could not replace the latest scan snapshot: {error}"))?;
    }
    fs::rename(&temporary, path)
        .map_err(|error| format!("Luna could not finish the latest scan snapshot: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::{AgeBuckets, ScanResult, StorageCategory},
        scanner::StorageIndex,
    };
    use std::collections::HashMap;

    #[test]
    fn round_trips_the_latest_detailed_scan() {
        let directory = std::env::temp_dir().join(format!(
            "luna-latest-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("latest-scan.json");
        let root = r"C:\Users\Luna".to_string();
        let area = StorageCategory {
            name: "Photos".to_string(),
            path: format!(r"{root}\Photos"),
            size_bytes: 42,
            file_count: 1,
            last_used_days: Some(2),
            can_drill_down: true,
        };
        let scan = ScanOutput {
            result: ScanResult {
                root: root.clone(),
                root_name: "Luna".to_string(),
                total_bytes: 42,
                drive_total_bytes: None,
                drive_used_bytes: None,
                file_count: 1,
                folder_count: 1,
                categories: vec![area.clone()],
                large_files: Vec::new(),
                duplicate_groups: Vec::new(),
                cleanup_items: Vec::new(),
                age_buckets: AgeBuckets::default(),
                scanned_at: "2026-07-14T12:00:00+02:00".to_string(),
                duration_ms: 10,
                warnings: Vec::new(),
                scan_method: "windows-directory".to_string(),
                snapshot_detail: None,
                snapshot_duplicate_reclaimable_bytes: None,
            },
            storage_index: StorageIndex {
                root: root.clone(),
                children: HashMap::from([(root.clone(), vec![area])]),
            },
        };

        save(&path, &scan).unwrap();
        let restored = load(&path).unwrap().unwrap();

        assert_eq!(restored.result.scanned_at, scan.result.scanned_at);
        assert_eq!(
            restored.storage_index.areas_for(&root).unwrap()[0].name,
            "Photos"
        );
        let _ = fs::remove_dir_all(directory);
    }
}
