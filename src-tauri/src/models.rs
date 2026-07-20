use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRootInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub scanned_files: u64,
    pub scanned_bytes: u64,
    pub drive_total_bytes: Option<u64>,
    pub drive_used_bytes: Option<u64>,
    pub current_path: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgeBuckets {
    pub recent_bytes: u64,
    pub inactive_30_to_90_bytes: u64,
    pub inactive_90_to_180_bytes: u64,
    pub inactive_180_plus_bytes: u64,
    pub unknown_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCategory {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub last_used_days: Option<u64>,
    pub can_drill_down: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub last_used_days: Option<u64>,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFileDeleteRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedLargeFile {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFileDeleteResult {
    pub removed_bytes: u64,
    pub removed_files: u64,
    pub deleted_files: Vec<DeletedLargeFile>,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateFile {
    pub name: String,
    pub path: String,
    pub last_used_days: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub content_hash: String,
    pub size_bytes: u64,
    pub reclaimable_bytes: u64,
    pub files: Vec<DuplicateFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupEvidenceSource {
    pub label: String,
    pub location: String,
    pub size_bytes: u64,
    pub file_count: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    pub id: String,
    pub group: String,
    pub name: String,
    pub source: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub last_used_days: Option<u64>,
    pub last_used_at: Option<String>,
    pub reason: String,
    pub detail: String,
    pub examples: String,
    pub confidence: String,
    pub selected_by_default: bool,
    pub evidence_count: usize,
    #[serde(default)]
    pub evidence_sources: Vec<CleanupEvidenceSource>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanPhaseTimings {
    pub inventory_ms: u128,
    pub duplicate_ms: u128,
    pub cleanup_ms: u128,
    pub finalize_ms: u128,
    pub snapshot_ms: u128,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub root: String,
    pub root_name: String,
    pub total_bytes: u64,
    pub drive_total_bytes: Option<u64>,
    pub drive_used_bytes: Option<u64>,
    pub file_count: u64,
    pub folder_count: u64,
    pub categories: Vec<StorageCategory>,
    pub large_files: Vec<LargeFile>,
    pub duplicate_groups: Vec<DuplicateGroup>,
    pub cleanup_items: Vec<CleanupItem>,
    pub age_buckets: AgeBuckets,
    pub scanned_at: String,
    pub duration_ms: u128,
    #[serde(default)]
    pub phase_timings: ScanPhaseTimings,
    pub warnings: Vec<String>,
    #[serde(default = "default_scan_method")]
    pub scan_method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_duplicate_reclaimable_bytes: Option<u64>,
}

fn default_scan_method() -> String {
    "windows-directory".to_string()
}

impl ScanResult {
    pub fn reported_used_bytes(&self) -> u64 {
        self.drive_used_bytes.unwrap_or(self.total_bytes)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRequest {
    pub item_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    pub removed_bytes: u64,
    pub removed_files: u64,
    pub failed_files: u64,
    pub completed_at: String,
    pub skipped: Vec<String>,
}
