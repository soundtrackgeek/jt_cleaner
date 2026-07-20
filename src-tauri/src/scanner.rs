use crate::{
    cloud_files::{self, CloudFilePolicy},
    models::{
        AgeBuckets, CleanupEvidenceSource, CleanupItem, CleanupResult, DeletedLargeFile,
        DuplicateFile, DuplicateGroup, LargeFile, LargeFileDeleteResult, ScanPhaseTimings,
        ScanProgress, ScanResult, ScanRootInfo, StorageCategory,
    },
};
use blake3::Hasher;
use chrono::{DateTime, Local, SecondsFormat};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet},
    env,
    fs::{self, File},
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};
use sysinfo::Disks;
use walkdir::{DirEntry, WalkDir};

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

const LARGE_FILE_LIMIT: usize = 40;
const LARGE_FILE_DELETE_LIMIT: usize = LARGE_FILE_LIMIT;
const DUPLICATE_CANDIDATE_LIMIT: usize = 20_000;
const DUPLICATE_SIZE_GROUP_LIMIT: usize = 60;
const DUPLICATE_FILES_PER_GROUP_LIMIT: usize = 12;
const MIN_DUPLICATE_SIZE: u64 = 1_048_576;
const DUPLICATE_SAMPLE_SIZE: usize = 65_536;

#[derive(Debug, Clone)]
struct CandidateFile {
    path: PathBuf,
    activity: Option<SystemTime>,
}

#[derive(Debug, Default)]
struct OneDriveScanStats {
    files: u64,
    online_only_files: u64,
    always_kept_files: u64,
    cached_files: u64,
}

impl OneDriveScanStats {
    fn observe(&mut self, online_only: bool, always_kept: bool) {
        self.files = self.files.saturating_add(1);
        if online_only {
            self.online_only_files = self.online_only_files.saturating_add(1);
        } else if always_kept {
            self.always_kept_files = self.always_kept_files.saturating_add(1);
        } else {
            self.cached_files = self.cached_files.saturating_add(1);
        }
    }
}

#[derive(Debug, Default)]
struct CategoryAccumulator {
    name: String,
    path: PathBuf,
    size_bytes: u64,
    file_count: u64,
    newest_activity: Option<SystemTime>,
    can_drill_down: bool,
}

type StorageAreaAccumulators = HashMap<PathBuf, HashMap<PathBuf, CategoryAccumulator>>;

#[derive(Debug, Clone, Copy, Default)]
struct DirectoryTotals {
    size_bytes: u64,
    file_count: u64,
    newest_activity: Option<SystemTime>,
}

impl DirectoryTotals {
    fn add_file(&mut self, size_bytes: u64, activity: Option<SystemTime>) {
        self.size_bytes = self.size_bytes.saturating_add(size_bytes);
        self.file_count = self.file_count.saturating_add(1);
        self.newest_activity = newest_time(self.newest_activity, activity);
    }

    fn add_directory(&mut self, child: Self) {
        self.size_bytes = self.size_bytes.saturating_add(child.size_bytes);
        self.file_count = self.file_count.saturating_add(child.file_count);
        self.newest_activity = newest_time(self.newest_activity, child.newest_activity);
    }
}

#[derive(Debug)]
struct NtfsDirectoryAccumulator {
    parent_record_number: Option<u64>,
    path: PathBuf,
    direct_files: DirectoryTotals,
}

#[derive(Debug)]
struct NtfsStorageAccumulator {
    root_record_number: u64,
    directories: HashMap<u64, NtfsDirectoryAccumulator>,
}

impl NtfsStorageAccumulator {
    fn new(root: &Path, root_record_number: u64) -> Self {
        Self {
            root_record_number,
            directories: HashMap::from([(
                root_record_number,
                NtfsDirectoryAccumulator {
                    parent_record_number: None,
                    path: root.to_path_buf(),
                    direct_files: DirectoryTotals::default(),
                },
            )]),
        }
    }

    fn reserve(&mut self, additional: usize) {
        self.directories.reserve(additional);
    }

    fn record_directory(&mut self, record_number: u64, parent_record_number: u64, path: &Path) {
        let parent_record_number =
            (record_number != self.root_record_number).then_some(parent_record_number);
        let directory =
            self.directories
                .entry(record_number)
                .or_insert_with(|| NtfsDirectoryAccumulator {
                    parent_record_number,
                    path: path.to_path_buf(),
                    direct_files: DirectoryTotals::default(),
                });
        directory.parent_record_number = parent_record_number;
        if directory.path != path {
            directory.path = path.to_path_buf();
        }
    }

    fn record_file(
        &mut self,
        parent_record_number: u64,
        parent_path: &Path,
        size_bytes: u64,
        activity: Option<SystemTime>,
    ) {
        self.directories
            .entry(parent_record_number)
            .or_insert_with(|| NtfsDirectoryAccumulator {
                parent_record_number: None,
                path: parent_path.to_path_buf(),
                direct_files: DirectoryTotals::default(),
            })
            .direct_files
            .add_file(size_bytes, activity);
    }

    fn into_storage_index(mut self, root: &Path, now: SystemTime) -> StorageIndex {
        if self.directories.iter().any(|(record_number, directory)| {
            *record_number != self.root_record_number && directory.parent_record_number.is_none()
        }) {
            let records_by_path: HashMap<PathBuf, u64> = self
                .directories
                .iter()
                .map(|(record_number, directory)| (directory.path.clone(), *record_number))
                .collect();

            for (record_number, directory) in &mut self.directories {
                if *record_number == self.root_record_number
                    || directory.parent_record_number.is_some()
                {
                    continue;
                }
                directory.parent_record_number = directory
                    .path
                    .parent()
                    .and_then(|parent| records_by_path.get(parent))
                    .copied();
            }
        }

        let mut totals: HashMap<u64, DirectoryTotals> = self
            .directories
            .iter()
            .map(|(record_number, directory)| (*record_number, directory.direct_files))
            .collect();
        let mut record_numbers: Vec<u64> = self.directories.keys().copied().collect();
        record_numbers.sort_by_key(|record_number| {
            Reverse(
                self.directories
                    .get(record_number)
                    .map_or(0, |directory| directory.path.components().count()),
            )
        });

        for record_number in &record_numbers {
            let Some(parent_record_number) = self
                .directories
                .get(record_number)
                .and_then(|directory| directory.parent_record_number)
            else {
                continue;
            };
            let child = totals.get(record_number).copied().unwrap_or_default();
            totals
                .entry(parent_record_number)
                .or_default()
                .add_directory(child);
        }

        let mut children: HashMap<String, Vec<StorageCategory>> =
            HashMap::with_capacity(self.directories.len());
        for (record_number, directory) in &self.directories {
            let directory_key = directory.path.to_string_lossy().to_string();
            let mut areas = Vec::new();
            if directory.direct_files.file_count > 0 {
                areas.push(StorageCategory {
                    name: if *record_number == self.root_record_number {
                        "Files at root".to_string()
                    } else {
                        "Files in this folder".to_string()
                    },
                    path: directory_key.clone(),
                    size_bytes: directory.direct_files.size_bytes,
                    file_count: directory.direct_files.file_count,
                    last_used_days: days_since(directory.direct_files.newest_activity, now),
                    can_drill_down: false,
                });
            }
            children.insert(directory_key, areas);
        }

        for (record_number, directory) in &self.directories {
            let Some(parent_record_number) = directory.parent_record_number else {
                continue;
            };
            let Some(parent) = self.directories.get(&parent_record_number) else {
                continue;
            };
            let total = totals.get(record_number).copied().unwrap_or_default();
            let parent_key = parent.path.to_string_lossy();
            if let Some(areas) = children.get_mut(parent_key.as_ref()) {
                areas.push(StorageCategory {
                    name: directory
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| directory.path.to_string_lossy().to_string()),
                    path: directory.path.to_string_lossy().to_string(),
                    size_bytes: total.size_bytes,
                    file_count: total.file_count,
                    last_used_days: days_since(total.newest_activity, now),
                    can_drill_down: true,
                });
            }
        }

        for areas in children.values_mut() {
            areas.sort_by(|left, right| right.size_bytes.cmp(&left.size_bytes));
        }

        StorageIndex {
            root: root.to_string_lossy().to_string(),
            children,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ScannedFileMetadata {
    logical_size: u64,
    local_size: u64,
    activity: Option<SystemTime>,
    online_only: bool,
    always_kept: bool,
}

impl ScannedFileMetadata {
    fn from_filesystem(metadata: &fs::Metadata) -> Self {
        Self {
            logical_size: metadata.len(),
            local_size: cloud_files::local_size_bytes(metadata),
            activity: activity_time(metadata),
            online_only: cloud_files::is_online_only(metadata),
            always_kept: cloud_files::is_always_kept(metadata),
        }
    }

    #[cfg(windows)]
    fn from_ntfs(logical_size: u64, activity: Option<SystemTime>, attributes: u32) -> Self {
        Self {
            logical_size,
            local_size: cloud_files::local_size_bytes_for_attributes(logical_size, attributes),
            activity,
            online_only: cloud_files::is_online_only_attributes(attributes),
            always_kept: cloud_files::is_always_kept_attributes(attributes),
        }
    }
}

#[derive(Debug)]
struct ScanAccumulator {
    now: SystemTime,
    total_bytes: u64,
    file_count: u64,
    folder_count: u64,
    storage_areas: StorageAreaAccumulators,
    ages: AgeBuckets,
    largest: BinaryHeap<Reverse<(u64, String, Option<SystemTime>)>>,
    duplicate_candidates: HashMap<u64, Vec<CandidateFile>>,
    duplicate_candidate_count: usize,
    warnings: Vec<String>,
    one_drive_stats: OneDriveScanStats,
}

impl ScanAccumulator {
    fn new(root: &Path, now: SystemTime) -> Self {
        let mut storage_areas = StorageAreaAccumulators::new();
        storage_areas.insert(root.to_path_buf(), HashMap::new());
        Self {
            now,
            total_bytes: 0,
            file_count: 0,
            folder_count: 0,
            storage_areas,
            ages: AgeBuckets::default(),
            largest: BinaryHeap::new(),
            duplicate_candidates: HashMap::new(),
            duplicate_candidate_count: 0,
            warnings: Vec::new(),
            one_drive_stats: OneDriveScanStats::default(),
        }
    }

    fn record_directory(&mut self, root: &Path, path: &Path) {
        self.folder_count = self.folder_count.saturating_add(1);
        record_storage_directory(root, path, &mut self.storage_areas);
    }

    fn record_ntfs_directory(
        &mut self,
        storage: &mut NtfsStorageAccumulator,
        record_number: u64,
        parent_record_number: u64,
        path: &Path,
    ) {
        self.folder_count = self.folder_count.saturating_add(1);
        storage.record_directory(record_number, parent_record_number, path);
    }

    fn record_ntfs_file<F>(
        &mut self,
        storage: &mut NtfsStorageAccumulator,
        parent_record_number: u64,
        parent_path: &Path,
        metadata: ScannedFileMetadata,
        is_one_drive: bool,
        materialize_path: F,
    ) where
        F: FnOnce() -> PathBuf,
    {
        self.total_bytes = self.total_bytes.saturating_add(metadata.local_size);
        self.file_count = self.file_count.saturating_add(1);
        add_age_bytes(
            &mut self.ages,
            metadata.local_size,
            metadata.activity,
            self.now,
        );
        storage.record_file(
            parent_record_number,
            parent_path,
            metadata.local_size,
            metadata.activity,
        );

        if is_one_drive {
            self.one_drive_stats
                .observe(metadata.online_only, metadata.always_kept);
        }

        let qualifies_as_large = metadata.local_size > 0
            && (self.largest.len() < LARGE_FILE_LIMIT
                || self
                    .largest
                    .peek()
                    .is_some_and(|Reverse((smallest, _, _))| metadata.local_size > *smallest));
        let qualifies_as_duplicate = metadata.logical_size >= MIN_DUPLICATE_SIZE
            && self.duplicate_candidate_count < DUPLICATE_CANDIDATE_LIMIT
            && !is_one_drive
            && !metadata.online_only;
        if !qualifies_as_large && !qualifies_as_duplicate {
            return;
        }

        let path = materialize_path();
        if qualifies_as_large {
            self.largest.push(Reverse((
                metadata.local_size,
                path.to_string_lossy().to_string(),
                metadata.activity,
            )));
            if self.largest.len() > LARGE_FILE_LIMIT {
                self.largest.pop();
            }
        }

        if qualifies_as_duplicate {
            self.duplicate_candidates
                .entry(metadata.logical_size)
                .or_default()
                .push(CandidateFile {
                    path,
                    activity: metadata.activity,
                });
            self.duplicate_candidate_count += 1;
        }
    }

    fn record_file(
        &mut self,
        root: &Path,
        path: &Path,
        metadata: ScannedFileMetadata,
        cloud_policy: &CloudFilePolicy,
    ) {
        self.total_bytes = self.total_bytes.saturating_add(metadata.local_size);
        self.file_count = self.file_count.saturating_add(1);
        add_age_bytes(
            &mut self.ages,
            metadata.local_size,
            metadata.activity,
            self.now,
        );
        record_storage_file(
            root,
            path,
            metadata.local_size,
            metadata.activity,
            &mut self.storage_areas,
        );

        let is_one_drive = cloud_policy.is_one_drive_path(path);
        if is_one_drive {
            self.one_drive_stats
                .observe(metadata.online_only, metadata.always_kept);
        }

        let qualifies_as_large = metadata.local_size > 0
            && (self.largest.len() < LARGE_FILE_LIMIT
                || self
                    .largest
                    .peek()
                    .is_some_and(|Reverse((smallest, _, _))| metadata.local_size > *smallest));
        if qualifies_as_large {
            let display_path = path.to_string_lossy().to_string();
            self.largest.push(Reverse((
                metadata.local_size,
                display_path,
                metadata.activity,
            )));
            if self.largest.len() > LARGE_FILE_LIMIT {
                self.largest.pop();
            }
        }

        if metadata.logical_size >= MIN_DUPLICATE_SIZE
            && self.duplicate_candidate_count < DUPLICATE_CANDIDATE_LIMIT
            && !is_one_drive
            && !metadata.online_only
        {
            self.duplicate_candidates
                .entry(metadata.logical_size)
                .or_default()
                .push(CandidateFile {
                    path: path.to_path_buf(),
                    activity: metadata.activity,
                });
            self.duplicate_candidate_count += 1;
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct StorageIndex {
    pub(crate) root: String,
    pub(crate) children: HashMap<String, Vec<StorageCategory>>,
}

impl StorageIndex {
    pub(crate) fn from_snapshot(root: &str, categories: &[StorageCategory]) -> Self {
        Self {
            root: root.to_string(),
            children: HashMap::from([(root.to_string(), categories.to_vec())]),
        }
    }

    fn from_accumulators(
        root: &Path,
        mut accumulators: StorageAreaAccumulators,
        now: SystemTime,
    ) -> Self {
        roll_up_storage_areas(root, &mut accumulators);
        let children = accumulators
            .into_iter()
            .map(|(parent, areas)| {
                let mut areas: Vec<StorageCategory> = areas
                    .into_values()
                    .map(|area| StorageCategory {
                        name: area.name,
                        path: area.path.to_string_lossy().to_string(),
                        size_bytes: area.size_bytes,
                        file_count: area.file_count,
                        last_used_days: days_since(area.newest_activity, now),
                        can_drill_down: area.can_drill_down,
                    })
                    .collect();
                areas.sort_by(|left, right| right.size_bytes.cmp(&left.size_bytes));
                (parent.to_string_lossy().to_string(), areas)
            })
            .collect();

        Self {
            root: root.to_string_lossy().to_string(),
            children,
        }
    }

    pub(crate) fn areas_for(&self, path: &str) -> Result<Vec<StorageCategory>, String> {
        if self.root.is_empty() {
            return Err("Run a scan before exploring folders.".to_string());
        }
        self.children.get(path).cloned().ok_or_else(|| {
            "That folder is not part of the current storage scan. Run the scan again and retry."
                .to_string()
        })
    }

    pub(crate) fn remove_files(&mut self, deleted: &[(String, u64)]) {
        for areas in self.children.values_mut() {
            for area in areas.iter_mut() {
                for (path, size_bytes) in deleted {
                    if storage_area_contains_file(&self.root, area, path) {
                        area.size_bytes = area.size_bytes.saturating_sub(*size_bytes);
                        area.file_count = area.file_count.saturating_sub(1);
                    }
                }
            }
            areas.retain(|area| area.file_count > 0 || area.can_drill_down);
            areas.sort_by(|left, right| right.size_bytes.cmp(&left.size_bytes));
        }
    }
}

fn storage_area_contains_file(root: &str, area: &StorageCategory, file: &str) -> bool {
    let root = Path::new(root);
    let area_path = Path::new(&area.path);
    let file_path = Path::new(file);
    if area_path == root {
        file_path.parent() == Some(root)
    } else {
        file_path.starts_with(area_path)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LargeFileMetadata {
    pub(crate) name: String,
    pub(crate) relative_path: String,
    pub(crate) extension: String,
    pub(crate) size_bytes: u64,
    pub(crate) last_used_days: Option<u64>,
    pub(crate) activity_at: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct LargeFileIndex {
    root: PathBuf,
    files: HashMap<String, LargeFile>,
}

impl LargeFileIndex {
    pub(crate) fn from_scan(root: &str, files: &[LargeFile]) -> Self {
        Self {
            root: PathBuf::from(root),
            files: files
                .iter()
                .cloned()
                .map(|file| (file.path.clone(), file))
                .collect(),
        }
    }

    pub(crate) fn metadata_for(&self, path: &str) -> Result<LargeFileMetadata, String> {
        let record = self.record_for(path)?;
        let canonical = self.validate_record(record)?;
        let relative_path = canonical
            .strip_prefix(&self.root)
            .map_err(|_| {
                "That file is outside the current scan root. Run the scan again and retry."
                    .to_string()
            })?
            .to_string_lossy()
            .to_string();
        let extension = canonical
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();

        Ok(LargeFileMetadata {
            name: record.name.clone(),
            relative_path,
            extension,
            size_bytes: record.size_bytes,
            last_used_days: record.last_used_days,
            activity_at: record.modified_at.clone(),
        })
    }

    pub(crate) fn delete_files(
        &mut self,
        paths: &[String],
    ) -> Result<LargeFileDeleteResult, String> {
        if paths.is_empty() {
            return Err("Select at least one large file to delete.".to_string());
        }
        if paths.len() > LARGE_FILE_DELETE_LIMIT {
            return Err(format!(
                "Delete at most {LARGE_FILE_DELETE_LIMIT} large files from one scan."
            ));
        }

        let mut seen = HashSet::new();
        let mut targets = Vec::new();
        let mut failed = Vec::new();
        for path in paths {
            if !seen.insert(path.clone()) {
                continue;
            }
            let record = self.record_for(path)?.clone();
            match self.validate_record(&record) {
                Ok(canonical) => targets.push((record, canonical)),
                Err(error) => failed.push(format!("{}: {error}", record.name)),
            }
        }

        let mut removed_bytes = 0_u64;
        let mut deleted_files = Vec::new();
        for (record, canonical) in targets {
            match fs::remove_file(&canonical) {
                Ok(()) => {
                    removed_bytes = removed_bytes.saturating_add(record.size_bytes);
                    deleted_files.push(DeletedLargeFile {
                        path: record.path.clone(),
                        size_bytes: record.size_bytes,
                    });
                    self.files.remove(&record.path);
                }
                Err(error) => failed.push(format!(
                    "{}: Windows could not delete it ({error}).",
                    record.name
                )),
            }
        }

        Ok(LargeFileDeleteResult {
            removed_bytes,
            removed_files: deleted_files.len() as u64,
            deleted_files,
            failed,
        })
    }

    pub(crate) fn remove_deleted(&mut self, deleted: &[(String, u64)]) {
        for (path, _) in deleted {
            self.files.remove(path);
        }
    }

    fn record_for(&self, path: &str) -> Result<&LargeFile, String> {
        if self.root.as_os_str().is_empty() {
            return Err("Run a scan before working with large files.".to_string());
        }
        self.files.get(path).ok_or_else(|| {
            "That file is not part of the current Large Files result. Run the scan again and retry."
                .to_string()
        })
    }

    fn validate_record(&self, record: &LargeFile) -> Result<PathBuf, String> {
        self.validate_record_with_policy(record, &CloudFilePolicy::from_environment())
    }

    fn validate_record_with_policy(
        &self,
        record: &LargeFile,
        cloud_policy: &CloudFilePolicy,
    ) -> Result<PathBuf, String> {
        let path = Path::new(&record.path);
        if cloud_policy.is_one_drive_path(path) {
            return Err(
                "Luna leaves OneDrive files untouched. Manage this file through OneDrive or File Explorer."
                    .to_string(),
            );
        }
        let metadata = fs::symlink_metadata(path).map_err(|_| {
            "It is no longer available. Run the scan again to refresh the list.".to_string()
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("It is no longer the regular file that Luna scanned.".to_string());
        }
        if metadata.len() != record.size_bytes {
            return Err(
                "Its size changed after the scan. Run the scan again before continuing."
                    .to_string(),
            );
        }

        let canonical = fs::canonicalize(path).map_err(|_| {
            "Windows could not revalidate its location. Run the scan again.".to_string()
        })?;
        if !canonical.starts_with(&self.root) || canonical != path {
            return Err(
                "Its location changed after the scan. Run the scan again before continuing."
                    .to_string(),
            );
        }
        Ok(canonical)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ScanOutput {
    pub(crate) result: ScanResult,
    pub(crate) storage_index: StorageIndex,
}

#[derive(Debug, Default, Clone)]
struct PathStats {
    size_bytes: u64,
    file_count: u64,
    newest_activity: Option<SystemTime>,
    oldest_activity: Option<SystemTime>,
}

#[derive(Debug, Clone)]
struct CacheTarget {
    category: &'static str,
    source: &'static str,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VolumeSpace {
    total_bytes: u64,
    available_bytes: u64,
}

impl VolumeSpace {
    fn used_bytes(self) -> u64 {
        self.total_bytes.saturating_sub(self.available_bytes)
    }
}

pub fn default_scan_root() -> Result<String, String> {
    dirs::home_dir()
        .map(|path| path.to_string_lossy().to_string())
        .ok_or_else(|| "Luna could not determine your home folder.".to_string())
}

pub fn list_scan_roots() -> Vec<ScanRootInfo> {
    let mut roots = Vec::new();

    if let Some(home) = dirs::home_dir() {
        roots.push(ScanRootInfo {
            id: "home".to_string(),
            name: "Home folder".to_string(),
            path: home.to_string_lossy().to_string(),
            total_bytes: 0,
            available_bytes: 0,
            kind: "home".to_string(),
        });
    }

    let disks = Disks::new_with_refreshed_list();
    for (index, disk) in disks.iter().enumerate() {
        let mount = disk.mount_point().to_string_lossy().to_string();
        let label = disk.name().to_string_lossy();
        let name = if label.trim().is_empty() {
            format!("Local Disk ({mount})")
        } else {
            format!("{} ({mount})", label.trim())
        };

        roots.push(ScanRootInfo {
            id: format!("disk-{index}"),
            name,
            path: mount,
            total_bytes: disk.total_space(),
            available_bytes: disk.available_space(),
            kind: if disk.is_removable() {
                "removable".to_string()
            } else {
                "fixed".to_string()
            },
        });
    }

    roots
}

#[cfg(windows)]
pub(crate) fn is_full_ntfs_volume_path(requested_path: &str) -> bool {
    let Ok(root) = fs::canonicalize(requested_path) else {
        return false;
    };
    if !crate::ntfs_scanner::is_volume_root(&root) {
        return false;
    }

    Disks::new_with_refreshed_list().iter().any(|disk| {
        paths_refer_to_same_location(&root, disk.mount_point())
            && disk
                .file_system()
                .to_string_lossy()
                .eq_ignore_ascii_case("ntfs")
    })
}

#[cfg(not(windows))]
pub(crate) fn is_full_ntfs_volume_path(_requested_path: &str) -> bool {
    false
}

fn volume_space_for_root(root: &Path) -> Option<VolumeSpace> {
    let disks = Disks::new_with_refreshed_list();
    volume_space_from_mounts(
        root,
        disks.iter().map(|disk| {
            (
                disk.mount_point(),
                disk.total_space(),
                disk.available_space(),
            )
        }),
    )
}

fn volume_space_from_mounts<'a>(
    root: &Path,
    mounts: impl IntoIterator<Item = (&'a Path, u64, u64)>,
) -> Option<VolumeSpace> {
    mounts
        .into_iter()
        .find(|(mount, _, _)| paths_refer_to_same_location(root, mount))
        .map(|(_, total_bytes, available_bytes)| VolumeSpace {
            total_bytes,
            available_bytes,
        })
}

fn paths_refer_to_same_location(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());

    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }

    #[cfg(not(windows))]
    {
        left == right
    }
}

fn scan_progress(
    scanned_files: u64,
    scanned_bytes: u64,
    current_path: &Path,
    volume_space: Option<VolumeSpace>,
) -> ScanProgress {
    ScanProgress {
        scanned_files,
        scanned_bytes,
        drive_total_bytes: volume_space.map(|space| space.total_bytes),
        drive_used_bytes: volume_space.map(VolumeSpace::used_bytes),
        current_path: current_path.to_string_lossy().to_string(),
    }
}

pub fn scan_path<F>(requested_path: &str, on_progress: F) -> Result<ScanOutput, String>
where
    F: FnMut(ScanProgress),
{
    scan_path_with_policy(
        requested_path,
        CloudFilePolicy::from_environment(),
        on_progress,
    )
}

fn scan_path_with_policy<F>(
    requested_path: &str,
    cloud_policy: CloudFilePolicy,
    mut on_progress: F,
) -> Result<ScanOutput, String>
where
    F: FnMut(ScanProgress),
{
    let started = Instant::now();
    let root = fs::canonicalize(requested_path)
        .map_err(|error| format!("Luna could not open {requested_path}: {error}"))?;

    if !root.is_dir() {
        return Err("Choose a folder or drive to scan.".to_string());
    }

    let progress_volume_space = volume_space_for_root(&root);
    on_progress(scan_progress(0, 0, &root, progress_volume_space));

    let now = SystemTime::now();
    let mut scan = ScanAccumulator::new(&root, now);
    let mut scan_method = "windows-directory".to_string();
    let mut fast_scan_complete = false;
    let mut ntfs_catalogue_read_ms = 0;
    let mut ntfs_record_fixup_ms = 0;
    let mut ntfs_record_parse_ms = 0;
    #[cfg(windows)]
    let mut ntfs_storage =
        NtfsStorageAccumulator::new(&root, crate::ntfs_scanner::NTFS_ROOT_RECORD);
    #[cfg(windows)]
    let mut ntfs_included_directories =
        HashMap::from([(crate::ntfs_scanner::NTFS_ROOT_RECORD, true)]);
    #[cfg(windows)]
    let mut ntfs_one_drive_directories = HashMap::from([(
        crate::ntfs_scanner::NTFS_ROOT_RECORD,
        cloud_policy.is_one_drive_path(&root),
    )]);
    #[cfg(windows)]
    let mut ntfs_capacity_reserved = false;

    #[cfg(windows)]
    match crate::ntfs_scanner::scan_volume(&root, |entry| {
        if !ntfs_capacity_reserved {
            ntfs_storage.reserve(entry.directory_capacity_hint);
            ntfs_included_directories.reserve(entry.directory_capacity_hint);
            ntfs_one_drive_directories.reserve(entry.directory_capacity_hint);
            ntfs_capacity_reserved = true;
        }

        let parent_is_included = *ntfs_included_directories
            .entry(entry.parent_record_number)
            .or_insert_with(|| should_include_ntfs_entry(&root, entry.parent_path, true, 0));
        if !parent_is_included {
            return;
        }

        if entry.is_directory {
            let Some(path) = entry.directory_path else {
                return;
            };
            let is_included = should_include_ntfs_entry(&root, path, true, entry.file_attributes);
            ntfs_included_directories.insert(entry.record_number, is_included);
            if !is_included {
                return;
            }
            ntfs_one_drive_directories
                .insert(entry.record_number, cloud_policy.is_one_drive_path(path));
            scan.record_ntfs_directory(
                &mut ntfs_storage,
                entry.record_number,
                entry.parent_record_number,
                path,
            );
            return;
        }

        let is_one_drive = *ntfs_one_drive_directories
            .entry(entry.parent_record_number)
            .or_insert_with(|| cloud_policy.is_one_drive_path(entry.parent_path));
        scan.record_ntfs_file(
            &mut ntfs_storage,
            entry.parent_record_number,
            entry.parent_path,
            ScannedFileMetadata::from_ntfs(
                entry.logical_size,
                entry.activity,
                entry.file_attributes,
            ),
            is_one_drive,
            || entry.materialize_path(),
        );
        if scan.file_count.is_multiple_of(1_000) {
            on_progress(scan_progress(
                scan.file_count,
                scan.total_bytes,
                entry.parent_path,
                progress_volume_space,
            ));
        }
    }) {
        Ok(Some(summary)) => {
            fast_scan_complete = true;
            scan_method = "ntfs-mft".to_string();
            ntfs_catalogue_read_ms = summary.catalogue_read_ms;
            ntfs_record_fixup_ms = summary.record_fixup_ms;
            ntfs_record_parse_ms = summary.record_parse_ms;
            if summary.used_compatibility_reader {
                push_warning(
                    &mut scan.warnings,
                    "Wide sequential NTFS reads were unavailable, so Luna used the compatible 4 KiB catalogue reader; inventory will be slower on this volume."
                        .to_string(),
                );
            }
            if summary.unresolved_records > 0 {
                push_warning(
                    &mut scan.warnings,
                    format!(
                        "The NTFS catalogue skipped {} active records whose current paths could not be resolved.",
                        summary.unresolved_records
                    ),
                );
            }
        }
        Ok(None) => {}
        Err(error) => push_warning(
            &mut scan.warnings,
            format!(
                "Fast NTFS catalogue scanning was unavailable, so Luna used Windows directory enumeration. {error}. An elevated full-volume NTFS scan is required for the fast path."
            ),
        ),
    }

    if !fast_scan_complete {
        let walker = WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(should_descend);

        for result in walker {
            let entry = match result {
                Ok(entry) => entry,
                Err(error) => {
                    push_warning(
                        &mut scan.warnings,
                        format!("Skipped an unreadable location: {error}"),
                    );
                    continue;
                }
            };

            if entry.depth() == 0 {
                continue;
            }

            if entry.file_type().is_dir() {
                scan.record_directory(&root, entry.path());
                continue;
            }

            if !entry.file_type().is_file() {
                continue;
            }

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    push_warning(
                        &mut scan.warnings,
                        format!("Skipped metadata for {}: {error}", entry.path().display()),
                    );
                    continue;
                }
            };

            scan.record_file(
                &root,
                entry.path(),
                ScannedFileMetadata::from_filesystem(&metadata),
                &cloud_policy,
            );

            if scan.file_count.is_multiple_of(1_000) {
                on_progress(scan_progress(
                    scan.file_count,
                    scan.total_bytes,
                    entry.path(),
                    progress_volume_space,
                ));
            }
        }
    }

    if scan.one_drive_stats.files > 0 {
        push_warning(
            &mut scan.warnings,
            format!(
                "Measured {} OneDrive files from metadata only: {} online-only files counted as 0 local bytes, {} always-kept files counted locally, and {} temporarily cached files counted while they occupy disk. Luna did not open, hash, download, or clean them.",
                scan.one_drive_stats.files,
                scan.one_drive_stats.online_only_files,
                scan.one_drive_stats.always_kept_files,
                scan.one_drive_stats.cached_files,
            ),
        );
    }

    if scan.duplicate_candidate_count == DUPLICATE_CANDIDATE_LIMIT {
        push_warning(
            &mut scan.warnings,
            "Duplicate analysis reached its 20,000-file safety limit; the storage totals remain complete."
                .to_string(),
        );
    }

    let inventory_ms = started.elapsed().as_millis();
    let duplicate_started = Instant::now();
    let duplicate_groups = find_duplicate_groups(
        scan.duplicate_candidates,
        now,
        &cloud_policy,
        &mut scan.warnings,
    );
    let duplicate_ms = duplicate_started.elapsed().as_millis();
    let cleanup_started = Instant::now();
    let cleanup_items = build_cleanup_items(&duplicate_groups, now, &mut scan.warnings);
    let cleanup_ms = cleanup_started.elapsed().as_millis();

    let finalize_started = Instant::now();
    #[cfg(windows)]
    let storage_index = if fast_scan_complete {
        ntfs_storage.into_storage_index(&root, now)
    } else {
        StorageIndex::from_accumulators(&root, scan.storage_areas, now)
    };
    #[cfg(not(windows))]
    let storage_index = StorageIndex::from_accumulators(&root, scan.storage_areas, now);
    let mut categories = storage_index.areas_for(&root.to_string_lossy())?;
    categories.truncate(24);

    let mut large_files: Vec<LargeFile> = scan
        .largest
        .into_iter()
        .map(|Reverse((size_bytes, path, activity))| LargeFile {
            name: Path::new(&path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone()),
            path,
            size_bytes,
            last_used_days: days_since(activity, now),
            modified_at: activity.map(format_time),
        })
        .collect();
    large_files.sort_by(|left, right| right.size_bytes.cmp(&left.size_bytes));

    on_progress(scan_progress(
        scan.file_count,
        scan.total_bytes,
        &root,
        progress_volume_space,
    ));

    let root_name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| root.to_string_lossy().to_string());

    let volume_space = volume_space_for_root(&root);
    let finalize_ms = finalize_started.elapsed().as_millis();
    let duration_ms = started.elapsed().as_millis();

    Ok(ScanOutput {
        result: ScanResult {
            root: root.to_string_lossy().to_string(),
            root_name,
            total_bytes: scan.total_bytes,
            drive_total_bytes: volume_space.map(|space| space.total_bytes),
            drive_used_bytes: volume_space.map(VolumeSpace::used_bytes),
            file_count: scan.file_count,
            folder_count: scan.folder_count,
            categories,
            large_files,
            duplicate_groups,
            cleanup_items,
            age_buckets: scan.ages,
            scanned_at: format_time(SystemTime::now()),
            duration_ms,
            phase_timings: ScanPhaseTimings {
                inventory_ms,
                ntfs_catalogue_read_ms,
                ntfs_record_fixup_ms,
                ntfs_record_parse_ms,
                duplicate_ms,
                cleanup_ms,
                finalize_ms,
                snapshot_ms: 0,
            },
            warnings: scan.warnings,
            scan_method,
            snapshot_detail: None,
            snapshot_duplicate_reclaimable_bytes: None,
        },
        storage_index,
    })
}

pub fn clean_items(item_ids: &[String]) -> CleanupResult {
    let now = SystemTime::now();
    let requested: HashSet<&str> = item_ids.iter().map(String::as_str).collect();
    let targets = known_cache_targets();
    let mut removed_bytes = 0_u64;
    let mut removed_files = 0_u64;
    let mut failed_files = 0_u64;
    let mut skipped = Vec::new();

    for item_id in &requested {
        if matches!(*item_id, "duplicate-files" | "old-downloads") {
            skipped.push(format!(
                "{item_id} requires file-by-file review and was not removed."
            ));
            continue;
        }

        if !matches!(*item_id, "browser-cache" | "codex-cache" | "temp-files") {
            skipped.push(format!("Unknown cleanup category: {item_id}."));
            continue;
        }

        let matching: Vec<&CacheTarget> = targets
            .iter()
            .filter(|target| target.category == *item_id)
            .collect();
        if matching.is_empty() {
            skipped.push(format!("No {item_id} paths are currently available."));
            continue;
        }

        for target in matching {
            if !is_safe_cleanup_target(target) {
                skipped.push(format!(
                    "Safety validation rejected {}.",
                    target.path.display()
                ));
                continue;
            }

            let entries = match fs::read_dir(&target.path) {
                Ok(entries) => entries,
                Err(error) => {
                    skipped.push(format!("Could not read {}: {error}", target.path.display()));
                    continue;
                }
            };

            for entry in entries.flatten() {
                remove_entry(
                    &entry.path(),
                    &mut removed_bytes,
                    &mut removed_files,
                    &mut failed_files,
                );
            }
        }
    }

    CleanupResult {
        removed_bytes,
        removed_files,
        failed_files,
        completed_at: format_time(now),
        skipped,
    }
}

fn should_descend(entry: &DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return true;
    }

    !is_excluded_directory_name(&entry.file_name().to_string_lossy())
}

fn is_excluded_directory_name(name: &str) -> bool {
    [
        "$extend",
        "$recycle.bin",
        "system volume information",
        "recovery",
        ".git",
        "node_modules",
    ]
    .iter()
    .any(|excluded| name.eq_ignore_ascii_case(excluded))
}

#[cfg(windows)]
fn should_include_ntfs_entry(
    root: &Path,
    path: &Path,
    is_directory: bool,
    file_attributes: u32,
) -> bool {
    if is_directory && file_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return false;
    }

    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let is_directory_component = is_directory || components.peek().is_some();
        if is_directory_component
            && is_excluded_directory_name(&component.as_os_str().to_string_lossy())
        {
            return false;
        }
    }
    true
}

fn record_storage_file(
    root: &Path,
    file: &Path,
    size: u64,
    activity: Option<SystemTime>,
    storage_areas: &mut StorageAreaAccumulators,
) {
    let Some(parent) = file.parent().filter(|parent| parent.starts_with(root)) else {
        return;
    };
    ensure_storage_directory(root, parent, storage_areas);

    let name = if parent == root {
        "Files at root".to_string()
    } else {
        "Files in this folder".to_string()
    };
    let area = storage_areas
        .entry(parent.to_path_buf())
        .or_default()
        .entry(parent.to_path_buf())
        .or_insert_with(|| CategoryAccumulator {
            name,
            path: parent.to_path_buf(),
            can_drill_down: false,
            ..CategoryAccumulator::default()
        });
    area.size_bytes = area.size_bytes.saturating_add(size);
    area.file_count = area.file_count.saturating_add(1);
    area.newest_activity = newest_time(area.newest_activity, activity);
}

fn record_storage_directory(
    root: &Path,
    directory: &Path,
    storage_areas: &mut StorageAreaAccumulators,
) {
    ensure_storage_directory(root, directory, storage_areas);
}

fn ensure_storage_directory(
    root: &Path,
    directory: &Path,
    storage_areas: &mut StorageAreaAccumulators,
) {
    if storage_areas.contains_key(directory) || !directory.starts_with(root) {
        return;
    }

    if directory != root {
        let Some(parent) = directory.parent().filter(|parent| parent.starts_with(root)) else {
            return;
        };
        ensure_storage_directory(root, parent, storage_areas);
    }

    storage_areas.entry(directory.to_path_buf()).or_default();
    let Some(parent) = directory.parent().filter(|parent| parent.starts_with(root)) else {
        return;
    };
    let name = directory
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| directory.to_string_lossy().to_string());
    storage_areas
        .entry(parent.to_path_buf())
        .or_default()
        .entry(directory.to_path_buf())
        .or_insert_with(|| CategoryAccumulator {
            name,
            path: directory.to_path_buf(),
            can_drill_down: true,
            ..CategoryAccumulator::default()
        });
}

fn roll_up_storage_areas(root: &Path, storage_areas: &mut StorageAreaAccumulators) {
    let mut directories: Vec<PathBuf> = storage_areas.keys().cloned().collect();
    directories.sort_by_key(|path| Reverse(path.components().count()));

    for directory in directories {
        if directory == root {
            continue;
        }
        let Some(parent) = directory.parent().filter(|parent| parent.starts_with(root)) else {
            continue;
        };
        let Some(areas) = storage_areas.get(&directory) else {
            continue;
        };
        let (size_bytes, file_count, newest_activity) = areas.values().fold(
            (0_u64, 0_u64, None),
            |(size_bytes, file_count, newest_activity), area| {
                (
                    size_bytes.saturating_add(area.size_bytes),
                    file_count.saturating_add(area.file_count),
                    newest_time(newest_activity, area.newest_activity),
                )
            },
        );

        if let Some(parent_area) = storage_areas
            .get_mut(parent)
            .and_then(|areas| areas.get_mut(&directory))
        {
            parent_area.size_bytes = size_bytes;
            parent_area.file_count = file_count;
            parent_area.newest_activity = newest_activity;
        }
    }
}

fn activity_time(metadata: &fs::Metadata) -> Option<SystemTime> {
    newest_time(metadata.modified().ok(), metadata.accessed().ok())
}

fn newest_time(left: Option<SystemTime>, right: Option<SystemTime>) -> Option<SystemTime> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn oldest_time(left: Option<SystemTime>, right: Option<SystemTime>) -> Option<SystemTime> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn days_since(activity: Option<SystemTime>, now: SystemTime) -> Option<u64> {
    activity.map(|time| now.duration_since(time).unwrap_or_default().as_secs() / 86_400)
}

fn add_age_bytes(
    buckets: &mut AgeBuckets,
    size: u64,
    activity: Option<SystemTime>,
    now: SystemTime,
) {
    match days_since(activity, now) {
        Some(0..=30) => buckets.recent_bytes = buckets.recent_bytes.saturating_add(size),
        Some(31..=90) => {
            buckets.inactive_30_to_90_bytes = buckets.inactive_30_to_90_bytes.saturating_add(size)
        }
        Some(91..=180) => {
            buckets.inactive_90_to_180_bytes = buckets.inactive_90_to_180_bytes.saturating_add(size)
        }
        Some(_) => {
            buckets.inactive_180_plus_bytes = buckets.inactive_180_plus_bytes.saturating_add(size)
        }
        None => buckets.unknown_bytes = buckets.unknown_bytes.saturating_add(size),
    }
}

fn find_duplicate_groups(
    candidates: HashMap<u64, Vec<CandidateFile>>,
    now: SystemTime,
    cloud_policy: &CloudFilePolicy,
    warnings: &mut Vec<String>,
) -> Vec<DuplicateGroup> {
    let mut size_groups: Vec<(u64, Vec<CandidateFile>)> = candidates
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .collect();
    size_groups.sort_by(|(left_size, left_files), (right_size, right_files)| {
        let left_waste = left_size.saturating_mul(left_files.len().saturating_sub(1) as u64);
        let right_waste = right_size.saturating_mul(right_files.len().saturating_sub(1) as u64);
        right_waste.cmp(&left_waste)
    });
    size_groups.truncate(DUPLICATE_SIZE_GROUP_LIMIT);

    let mut duplicate_groups = Vec::new();
    for (size, files) in size_groups {
        let mut sample_hashes: HashMap<String, Vec<CandidateFile>> = HashMap::new();
        for candidate in files.into_iter().take(DUPLICATE_FILES_PER_GROUP_LIMIT) {
            match sample_file_with_policy(&candidate.path, size, cloud_policy) {
                Ok(hash) => sample_hashes.entry(hash).or_default().push(candidate),
                Err(error) => push_warning(
                    warnings,
                    format!("Could not compare {}: {error}", candidate.path.display()),
                ),
            }
        }

        let mut hashes: HashMap<String, Vec<CandidateFile>> = HashMap::new();
        for files in sample_hashes.into_values().filter(|files| files.len() > 1) {
            for candidate in files {
                match hash_file_with_policy_at_size(&candidate.path, size, cloud_policy) {
                    Ok(hash) => hashes.entry(hash).or_default().push(candidate),
                    Err(error) => push_warning(
                        warnings,
                        format!("Could not compare {}: {error}", candidate.path.display()),
                    ),
                }
            }
        }

        for (content_hash, files) in hashes {
            if files.len() < 2 {
                continue;
            }

            duplicate_groups.push(DuplicateGroup {
                content_hash,
                size_bytes: size,
                reclaimable_bytes: size.saturating_mul(files.len().saturating_sub(1) as u64),
                files: files
                    .into_iter()
                    .map(|file| DuplicateFile {
                        name: file
                            .path
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| file.path.to_string_lossy().to_string()),
                        path: file.path.to_string_lossy().to_string(),
                        last_used_days: days_since(file.activity, now),
                    })
                    .collect(),
            });
        }
    }

    duplicate_groups.sort_by(|left, right| right.reclaimable_bytes.cmp(&left.reclaimable_bytes));
    duplicate_groups.truncate(20);
    duplicate_groups
}

pub(crate) fn hash_file(path: &Path) -> Result<String, String> {
    hash_file_with_policy(path, &CloudFilePolicy::from_environment())
}

fn hash_file_with_policy(path: &Path, cloud_policy: &CloudFilePolicy) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if let Some(error) = cloud_policy.content_access_error(path, &metadata) {
        return Err(error.to_string());
    }
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(file);
    let mut hasher = Hasher::new();
    let mut buffer = [0_u8; 65_536];

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_file_with_policy_at_size(
    path: &Path,
    expected_size: u64,
    cloud_policy: &CloudFilePolicy,
) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() != expected_size {
        return Err("The file changed size after it was indexed.".to_string());
    }
    hash_file_with_policy(path, cloud_policy)
}

fn sample_file_with_policy(
    path: &Path,
    expected_size: u64,
    cloud_policy: &CloudFilePolicy,
) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() != expected_size {
        return Err("The file changed size after it was indexed.".to_string());
    }
    if let Some(error) = cloud_policy.content_access_error(path, &metadata) {
        return Err(error.to_string());
    }

    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let sample_size = expected_size.min(DUPLICATE_SAMPLE_SIZE as u64);
    let offsets = [
        0,
        expected_size.saturating_sub(sample_size) / 2,
        expected_size.saturating_sub(sample_size),
    ];
    let mut previous_offset = None;
    let mut buffer = [0_u8; DUPLICATE_SAMPLE_SIZE];
    let mut hasher = Hasher::new();
    hasher.update(&expected_size.to_le_bytes());

    for offset in offsets {
        if previous_offset == Some(offset) {
            continue;
        }
        previous_offset = Some(offset);
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        let mut read_total = 0;
        while read_total < sample_size as usize {
            let read = file
                .read(&mut buffer[read_total..sample_size as usize])
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("The file changed while Luna sampled it.".to_string());
            }
            read_total += read;
        }
        hasher.update(&offset.to_le_bytes());
        hasher.update(&buffer[..read_total]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

fn build_cleanup_items(
    duplicates: &[DuplicateGroup],
    now: SystemTime,
    warnings: &mut Vec<String>,
) -> Vec<CleanupItem> {
    let targets = known_cache_targets();
    let browser: Vec<&CacheTarget> = targets
        .iter()
        .filter(|target| target.category == "browser-cache")
        .collect();
    let codex: Vec<&CacheTarget> = targets
        .iter()
        .filter(|target| target.category == "codex-cache")
        .collect();
    let temporary: Vec<&CacheTarget> = targets
        .iter()
        .filter(|target| target.category == "temp-files")
        .collect();

    let (browser_stats, browser_evidence) = collect_target_evidence(&browser, warnings);
    let (codex_stats, codex_evidence) = collect_target_evidence(&codex, warnings);
    let (temp_stats, temp_evidence) = collect_target_evidence(&temporary, warnings);
    let old_download_path = dirs::download_dir();
    let old_download_stats = old_download_path
        .as_deref()
        .map(|path| scan_old_downloads(path, now, warnings))
        .unwrap_or_default();
    let old_download_evidence = old_download_path
        .as_deref()
        .map(|path| evidence_source("Downloads", path, &old_download_stats))
        .into_iter()
        .collect();
    let duplicate_bytes = duplicates.iter().fold(0_u64, |sum, group| {
        sum.saturating_add(group.reclaimable_bytes)
    });
    let duplicate_files = duplicates
        .iter()
        .map(|group| group.files.len().saturating_sub(1) as u64)
        .sum();

    vec![
        cleanup_item(
            "browser-cache",
            "safe",
            "Browser cache",
            source_list(&browser, "Supported browsers"),
            browser_stats,
            now,
            "Temporary browser data that is recreated as needed.",
            "Luna only targets known browser cache folders. Bookmarks, passwords, history, profiles, and settings are excluded.",
            "Cached images, scripts, favicons, code cache, and GPU cache.",
            "High",
            true,
            browser_evidence,
        ),
        cleanup_item(
            "codex-cache",
            "safe",
            "Codex cache",
            "OpenAI Codex".to_string(),
            codex_stats,
            now,
            "Disposable cache and temporary files that Codex can recreate.",
            "Only cache, .cache, and tmp folders inside CODEX_HOME are eligible. Threads, configuration, skills, logs, and projects are excluded.",
            "Temporary downloads, cached generated data, and disposable runtime files.",
            "High",
            true,
            codex_evidence,
        ),
        cleanup_item(
            "temp-files",
            "safe",
            "Temporary files",
            "Windows user temp".to_string(),
            temp_stats,
            now,
            "Temporary files left by applications and installers.",
            "Luna attempts each item independently and leaves open or protected files untouched.",
            "Expired extraction folders, transient logs, and application scratch files.",
            "High",
            true,
            temp_evidence,
        ),
        CleanupItem {
            id: "duplicate-files".to_string(),
            group: "review".to_string(),
            name: "Duplicate files".to_string(),
            source: "Content hash matches".to_string(),
            size_bytes: duplicate_bytes,
            file_count: duplicate_files,
            last_used_days: None,
            last_used_at: None,
            reason: "Byte-identical files were found in more than one location.".to_string(),
            detail: "Matching BLAKE3 content hashes identify exact duplicates. Luna never assumes which copy you want to keep.".to_string(),
            examples: "Installers, archives, exports, and copied project assets.".to_string(),
            confidence: "Medium".to_string(),
            selected_by_default: false,
            evidence_count: duplicates.len(),
            evidence_sources: duplicate_evidence_sources(duplicates),
        },
        cleanup_item(
            "old-downloads",
            "review",
            "Downloads not used in 90+ days",
            "Your Downloads folder".to_string(),
            old_download_stats,
            now,
            "Older downloads have no recent activity signal and may no longer be needed.",
            "Windows last-access timestamps can be incomplete, so Luna also considers modification time. Age is evidence for review, never proof that a file is disposable.",
            "Archives, media exports, documents, and installers.",
            "Low",
            false,
            old_download_evidence,
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn cleanup_item(
    id: &str,
    group: &str,
    name: &str,
    source: String,
    stats: PathStats,
    now: SystemTime,
    reason: &str,
    detail: &str,
    examples: &str,
    confidence: &str,
    selected_by_default: bool,
    evidence_sources: Vec<CleanupEvidenceSource>,
) -> CleanupItem {
    let evidence_count = evidence_sources.len();
    CleanupItem {
        id: id.to_string(),
        group: group.to_string(),
        name: name.to_string(),
        source,
        size_bytes: stats.size_bytes,
        file_count: stats.file_count,
        last_used_days: days_since(stats.newest_activity, now),
        last_used_at: stats.newest_activity.map(format_time),
        reason: reason.to_string(),
        detail: detail.to_string(),
        examples: examples.to_string(),
        confidence: confidence.to_string(),
        selected_by_default: selected_by_default && stats.size_bytes > 0,
        evidence_count,
        evidence_sources,
    }
}

fn known_cache_targets() -> Vec<CacheTarget> {
    let mut targets = Vec::new();

    if let Some(local) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        for (relative, source) in [
            ("Google/Chrome/User Data", "Google Chrome"),
            ("Microsoft/Edge/User Data", "Microsoft Edge"),
            ("BraveSoftware/Brave-Browser/User Data", "Brave"),
        ] {
            add_chromium_profile_caches(&mut targets, &local.join(relative), source);
        }

        let firefox_profiles = local.join("Mozilla/Firefox/Profiles");
        add_profile_children(
            &mut targets,
            &firefox_profiles,
            &["cache2"],
            "Mozilla Firefox",
        );

        let opera = local.join("Opera Software/Opera Stable");
        for child in ["Cache", "Code Cache", "GPUCache"] {
            add_target_if_dir(&mut targets, "browser-cache", "Opera", opera.join(child));
        }
    }

    if let Some(codex_home) = codex_home() {
        for child in ["cache", ".cache", "tmp"] {
            add_target_if_dir(
                &mut targets,
                "codex-cache",
                "OpenAI Codex",
                codex_home.join(child),
            );
        }
    }

    add_target_if_dir(
        &mut targets,
        "temp-files",
        "Windows user temp",
        env::temp_dir(),
    );

    let mut seen = HashSet::new();
    targets.retain(|target| {
        let key = fs::canonicalize(&target.path).unwrap_or_else(|_| target.path.clone());
        seen.insert(key)
    });
    targets
}

fn add_chromium_profile_caches(
    targets: &mut Vec<CacheTarget>,
    user_data: &Path,
    source: &'static str,
) {
    add_profile_children(
        targets,
        user_data,
        &["Cache/Cache_Data", "Cache", "Code Cache", "GPUCache"],
        source,
    );
}

fn add_profile_children(
    targets: &mut Vec<CacheTarget>,
    parent: &Path,
    cache_paths: &[&str],
    source: &'static str,
) {
    let Ok(profiles) = fs::read_dir(parent) else {
        return;
    };

    for profile in profiles.flatten().filter(|entry| entry.path().is_dir()) {
        for cache_path in cache_paths {
            add_target_if_dir(
                targets,
                "browser-cache",
                source,
                profile.path().join(cache_path),
            );
        }
    }
}

fn add_target_if_dir(
    targets: &mut Vec<CacheTarget>,
    category: &'static str,
    source: &'static str,
    path: PathBuf,
) {
    if path.is_dir() {
        targets.push(CacheTarget {
            category,
            source,
            path,
        });
    }
}

fn codex_home() -> Option<PathBuf> {
    env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
}

fn collect_target_evidence(
    targets: &[&CacheTarget],
    warnings: &mut Vec<String>,
) -> (PathStats, Vec<CleanupEvidenceSource>) {
    let mut combined = PathStats::default();
    let mut evidence = Vec::with_capacity(targets.len());
    for target in targets {
        let stats = collect_path_stats(&target.path, warnings);
        combined.size_bytes = combined.size_bytes.saturating_add(stats.size_bytes);
        combined.file_count = combined.file_count.saturating_add(stats.file_count);
        combined.newest_activity = newest_time(combined.newest_activity, stats.newest_activity);
        combined.oldest_activity = oldest_time(combined.oldest_activity, stats.oldest_activity);
        evidence.push(evidence_source(target.source, &target.path, &stats));
    }
    (combined, evidence)
}

fn evidence_source(label: &str, path: &Path, stats: &PathStats) -> CleanupEvidenceSource {
    CleanupEvidenceSource {
        label: label.to_string(),
        location: path.to_string_lossy().to_string(),
        size_bytes: stats.size_bytes,
        file_count: stats.file_count,
    }
}

fn duplicate_evidence_sources(duplicates: &[DuplicateGroup]) -> Vec<CleanupEvidenceSource> {
    duplicates
        .iter()
        .map(|group| {
            let hash = group.content_hash.get(..8).unwrap_or(&group.content_hash);
            let location = match group.files.as_slice() {
                [] => "No file locations recorded".to_string(),
                [file] => file.path.clone(),
                [first, rest @ ..] => format!(
                    "{} and {} more {}",
                    first.path,
                    rest.len(),
                    if rest.len() == 1 {
                        "location"
                    } else {
                        "locations"
                    }
                ),
            };
            CleanupEvidenceSource {
                label: format!("Exact match {hash}"),
                location,
                size_bytes: group.reclaimable_bytes,
                file_count: group.files.len() as u64,
            }
        })
        .collect()
}

fn collect_path_stats(path: &Path, warnings: &mut Vec<String>) -> PathStats {
    let mut stats = PathStats::default();
    for entry in WalkDir::new(path).follow_links(false).into_iter() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                push_warning(warnings, format!("Skipped a cache entry: {error}"));
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let activity = activity_time(&metadata);
        stats.size_bytes = stats.size_bytes.saturating_add(metadata.len());
        stats.file_count = stats.file_count.saturating_add(1);
        stats.newest_activity = newest_time(stats.newest_activity, activity);
        stats.oldest_activity = oldest_time(stats.oldest_activity, activity);
    }
    stats
}

fn scan_old_downloads(downloads: &Path, now: SystemTime, warnings: &mut Vec<String>) -> PathStats {
    let mut stats = PathStats::default();
    for entry in WalkDir::new(downloads).follow_links(false).into_iter() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                push_warning(warnings, format!("Skipped a Downloads entry: {error}"));
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let activity = activity_time(&metadata);
        if days_since(activity, now).is_some_and(|days| days > 90) {
            stats.size_bytes = stats.size_bytes.saturating_add(metadata.len());
            stats.file_count = stats.file_count.saturating_add(1);
            stats.newest_activity = newest_time(stats.newest_activity, activity);
            stats.oldest_activity = oldest_time(stats.oldest_activity, activity);
        }
    }
    stats
}

fn source_list(targets: &[&CacheTarget], fallback: &str) -> String {
    let mut sources: Vec<&str> = targets.iter().map(|target| target.source).collect();
    sources.sort_unstable();
    sources.dedup();
    if sources.is_empty() {
        fallback.to_string()
    } else {
        sources.join(", ")
    }
}

fn is_safe_cleanup_target(target: &CacheTarget) -> bool {
    let Ok(path) = fs::canonicalize(&target.path) else {
        return false;
    };
    if !path.is_dir() || path.parent().is_none() {
        return false;
    }

    match target.category {
        "browser-cache" => {
            let Some(local) = env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .and_then(|path| fs::canonicalize(path).ok())
            else {
                return false;
            };
            let cache_component = path.components().any(|component| {
                matches!(
                    component
                        .as_os_str()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .as_str(),
                    "cache" | "cache_data" | "code cache" | "gpucache" | "cache2"
                )
            });
            path.starts_with(&local) && path != local && cache_component
        }
        "codex-cache" => {
            let Some(home) = codex_home().and_then(|path| fs::canonicalize(path).ok()) else {
                return false;
            };
            let Ok(relative) = path.strip_prefix(&home) else {
                return false;
            };
            let first = relative
                .components()
                .next()
                .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase());
            path != home && matches!(first.as_deref(), Some("cache" | ".cache" | "tmp"))
        }
        "temp-files" => fs::canonicalize(env::temp_dir()).is_ok_and(|temp| path == temp),
        _ => false,
    }
}

fn remove_entry(path: &Path, removed_bytes: &mut u64, removed_files: &mut u64, failed: &mut u64) {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => {
            *failed = failed.saturating_add(1);
            return;
        }
    };

    if metadata.file_type().is_symlink() {
        let result = if metadata.is_dir() {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        };
        if result.is_err() {
            *failed = failed.saturating_add(1);
        }
        return;
    }

    if metadata.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            *failed = failed.saturating_add(1);
            return;
        };
        for entry in entries.flatten() {
            remove_entry(&entry.path(), removed_bytes, removed_files, failed);
        }
        let _ = fs::remove_dir(path);
        return;
    }

    let size = metadata.len();
    match fs::remove_file(path) {
        Ok(()) => {
            *removed_bytes = removed_bytes.saturating_add(size);
            *removed_files = removed_files.saturating_add(1);
        }
        Err(_) => *failed = failed.saturating_add(1),
    }
}

fn format_time(time: SystemTime) -> String {
    let local: DateTime<Local> = time.into();
    local.to_rfc3339_opts(SecondsFormat::Secs, false)
}

fn push_warning(warnings: &mut Vec<String>, warning: String) {
    if warnings.len() < 12 && !warnings.contains(&warning) {
        warnings.push(warning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn age_buckets_follow_review_thresholds() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(400 * 86_400);
        let mut buckets = AgeBuckets::default();

        add_age_bytes(
            &mut buckets,
            10,
            Some(now - Duration::from_secs(12 * 86_400)),
            now,
        );
        add_age_bytes(
            &mut buckets,
            20,
            Some(now - Duration::from_secs(60 * 86_400)),
            now,
        );
        add_age_bytes(
            &mut buckets,
            30,
            Some(now - Duration::from_secs(120 * 86_400)),
            now,
        );
        add_age_bytes(
            &mut buckets,
            40,
            Some(now - Duration::from_secs(220 * 86_400)),
            now,
        );

        assert_eq!(buckets.recent_bytes, 10);
        assert_eq!(buckets.inactive_30_to_90_bytes, 20);
        assert_eq!(buckets.inactive_90_to_180_bytes, 30);
        assert_eq!(buckets.inactive_180_plus_bytes, 40);
    }

    #[test]
    fn cleanup_rejects_review_and_unknown_categories() {
        let result = clean_items(&["old-downloads".to_string(), "arbitrary-path".to_string()]);
        assert_eq!(result.removed_files, 0);
        assert_eq!(result.skipped.len(), 2);
    }

    #[test]
    fn one_drive_files_are_refused_before_content_hashing() {
        let root = temporary_large_file_root("onedrive-content-guard");
        let one_drive = root.join("OneDrive");
        fs::create_dir_all(&one_drive).expect("temporary OneDrive root should be created");
        let path = one_drive.join("online.raw");
        fs::write(&path, b"cloud contents must stay unopened").expect("temporary cloud file");
        let policy = CloudFilePolicy::from_roots([one_drive]);

        let result = hash_file_with_policy(&path, &policy);

        assert!(result.is_err_and(|error| error.contains("metadata-only")));
        fs::remove_dir_all(root).expect("temporary OneDrive root should be removed");
    }

    #[test]
    fn duplicate_sampling_rejects_same_sized_non_matches_before_full_hashing() {
        let root = temporary_large_file_root("duplicate-sampling");
        fs::create_dir_all(&root).expect("temporary duplicate root should be created");
        let first = root.join("first.bin");
        let second = root.join("second.bin");
        let different = root.join("different.bin");
        let contents = vec![b'a'; MIN_DUPLICATE_SIZE as usize];
        let mut different_contents = contents.clone();
        different_contents[contents.len() / 2] = b'b';
        fs::write(&first, &contents).expect("first duplicate should be written");
        fs::write(&second, &contents).expect("second duplicate should be written");
        fs::write(&different, &different_contents).expect("non-match should be written");
        let policy = CloudFilePolicy::default();

        let first_sample = sample_file_with_policy(&first, MIN_DUPLICATE_SIZE, &policy).unwrap();
        let second_sample = sample_file_with_policy(&second, MIN_DUPLICATE_SIZE, &policy).unwrap();
        let different_sample =
            sample_file_with_policy(&different, MIN_DUPLICATE_SIZE, &policy).unwrap();
        assert_eq!(first_sample, second_sample);
        assert_ne!(first_sample, different_sample);

        let candidates = HashMap::from([(
            MIN_DUPLICATE_SIZE,
            vec![
                CandidateFile {
                    path: first,
                    activity: None,
                },
                CandidateFile {
                    path: second,
                    activity: None,
                },
                CandidateFile {
                    path: different,
                    activity: None,
                },
            ],
        )]);
        let mut warnings = Vec::new();
        let groups = find_duplicate_groups(candidates, SystemTime::now(), &policy, &mut warnings);
        assert!(warnings.is_empty());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].files.len(), 2);

        fs::remove_dir_all(root).expect("temporary duplicate root should be removed");
    }

    #[test]
    fn one_drive_large_file_actions_are_refused() {
        let root = temporary_large_file_root("onedrive-large-file-guard");
        let one_drive = root.join("OneDrive");
        fs::create_dir_all(&one_drive).expect("temporary OneDrive root should be created");
        let path = one_drive.join("cached.raw");
        fs::write(&path, b"locally cached cloud file").expect("temporary cloud file");
        let canonical_root = fs::canonicalize(&root).expect("canonical temporary root");
        let canonical_one_drive = fs::canonicalize(&one_drive).expect("canonical OneDrive root");
        let canonical_path = fs::canonicalize(&path).expect("canonical cloud file");
        let display_path = canonical_path.to_string_lossy().to_string();
        let record = LargeFile {
            name: "cached.raw".to_string(),
            path: display_path.clone(),
            size_bytes: 25,
            last_used_days: Some(1),
            modified_at: None,
        };
        let index = LargeFileIndex::from_scan(&canonical_root.to_string_lossy(), &[record]);
        let policy = CloudFilePolicy::from_roots([canonical_one_drive]);

        let result = index.validate_record_with_policy(
            index.record_for(&display_path).expect("indexed cloud file"),
            &policy,
        );

        assert!(result.is_err_and(|error| error.contains("leaves OneDrive files untouched")));
        assert!(path.exists());
        fs::remove_dir_all(root).expect("temporary OneDrive root should be removed");
    }

    #[test]
    fn cleanup_evidence_reports_each_location_and_combined_totals() {
        let root = temporary_large_file_root("cleanup-evidence");
        let chrome = root.join("Chrome").join("Cache");
        let edge = root.join("Edge").join("Cache");
        fs::create_dir_all(&chrome).expect("Chrome cache should be created");
        fs::create_dir_all(&edge).expect("Edge cache should be created");
        fs::write(chrome.join("cached-image"), b"abc").expect("Chrome cache file");
        fs::write(edge.join("cached-script"), b"12345").expect("Edge cache file");
        let targets = [
            CacheTarget {
                category: "browser-cache",
                source: "Google Chrome",
                path: chrome.clone(),
            },
            CacheTarget {
                category: "browser-cache",
                source: "Microsoft Edge",
                path: edge.clone(),
            },
        ];
        let target_refs = targets.iter().collect::<Vec<_>>();
        let mut warnings = Vec::new();

        let (combined, evidence) = collect_target_evidence(&target_refs, &mut warnings);

        assert!(warnings.is_empty());
        assert_eq!(combined.size_bytes, 8);
        assert_eq!(combined.file_count, 2);
        assert_eq!(evidence.len(), 2);
        assert_eq!(evidence[0].label, "Google Chrome");
        assert_eq!(evidence[0].location, chrome.to_string_lossy());
        assert_eq!(evidence[0].size_bytes, 3);
        assert_eq!(evidence[1].label, "Microsoft Edge");
        assert_eq!(evidence[1].location, edge.to_string_lossy());
        assert_eq!(evidence[1].size_bytes, 5);
        fs::remove_dir_all(root).expect("temporary evidence root should be removed");
    }

    #[test]
    fn whole_drive_usage_comes_from_the_matching_volume() {
        let root = env::current_dir().expect("current directory");
        let canonical_root = fs::canonicalize(&root).expect("canonical current directory");
        let mounts = [(canonical_root.as_path(), 500, 125)];

        let space = volume_space_from_mounts(&root, mounts).expect("matching volume");

        assert_eq!(space.total_bytes, 500);
        assert_eq!(space.used_bytes(), 375);
        assert_eq!(
            volume_space_from_mounts(Path::new("definitely-not-the-root"), mounts),
            None
        );
    }

    #[test]
    fn drive_progress_reports_volume_usage_instead_of_logical_bytes() {
        let progress = scan_progress(
            1_337_000,
            706,
            Path::new("test-volume"),
            Some(VolumeSpace {
                total_bytes: 475,
                available_bytes: 41,
            }),
        );

        assert_eq!(progress.scanned_bytes, 706);
        assert_eq!(progress.drive_total_bytes, Some(475));
        assert_eq!(progress.drive_used_bytes, Some(434));
    }

    #[cfg(windows)]
    #[test]
    fn ntfs_inventory_keeps_files_but_prunes_excluded_and_reparse_directories() {
        let root = Path::new(r"C:\");

        assert!(should_include_ntfs_entry(
            root,
            Path::new(r"C:\Users\Alice\report.pdf"),
            false,
            0,
        ));
        assert!(should_include_ntfs_entry(
            root,
            Path::new(r"C:\Users\Alice\.git"),
            false,
            0,
        ));
        assert!(!should_include_ntfs_entry(
            root,
            Path::new(r"C:\Users\Alice\.git\objects\pack.bin"),
            false,
            0,
        ));
        assert!(!should_include_ntfs_entry(
            root,
            Path::new(r"C:\$Extend\$Reparse"),
            false,
            0,
        ));
        assert!(!should_include_ntfs_entry(
            root,
            Path::new(r"C:\MountedVolume"),
            true,
            FILE_ATTRIBUTE_REPARSE_POINT,
        ));
    }

    #[test]
    fn storage_index_groups_every_immediate_folder_level() {
        let root = PathBuf::from("scan-root");
        let alice = root.join("Users").join("Alice");
        let bob = root.join("Users").join("Bob");
        let empty = root.join("Users").join("Empty");
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10 * 86_400);
        let mut accumulators = StorageAreaAccumulators::new();
        accumulators.insert(root.clone(), HashMap::new());

        record_storage_file(
            &root,
            &alice.join("Documents").join("report.txt"),
            70,
            Some(now),
            &mut accumulators,
        );
        record_storage_file(
            &root,
            &bob.join("photo.jpg"),
            30,
            Some(now),
            &mut accumulators,
        );
        record_storage_file(
            &root,
            &root.join("pagefile.sys"),
            20,
            Some(now),
            &mut accumulators,
        );
        record_storage_directory(&root, &empty, &mut accumulators);

        let index = StorageIndex::from_accumulators(&root, accumulators, now);
        let root_areas = index.areas_for(&root.to_string_lossy()).unwrap();
        let users = root_areas.iter().find(|area| area.name == "Users").unwrap();
        assert_eq!(users.size_bytes, 100);
        assert!(users.can_drill_down);
        assert!(
            root_areas
                .iter()
                .any(|area| area.name == "Files at root" && !area.can_drill_down)
        );

        let user_areas = index
            .areas_for(&root.join("Users").to_string_lossy())
            .unwrap();
        assert_eq!(user_areas.len(), 3);
        assert_eq!(user_areas[0].name, "Alice");
        assert_eq!(user_areas[0].size_bytes, 70);
        assert!(
            user_areas
                .iter()
                .any(|area| area.name == "Empty" && area.size_bytes == 0 && area.can_drill_down)
        );
        assert!(
            index
                .areas_for(&empty.to_string_lossy())
                .unwrap()
                .is_empty()
        );

        let alice_areas = index.areas_for(&alice.to_string_lossy()).unwrap();
        assert_eq!(alice_areas[0].name, "Documents");
        let bob_areas = index.areas_for(&bob.to_string_lossy()).unwrap();
        assert_eq!(bob_areas[0].name, "Files in this folder");
        assert!(!bob_areas[0].can_drill_down);
    }

    #[test]
    fn ntfs_storage_index_rolls_up_record_ids_even_when_files_arrive_first() {
        let root = PathBuf::from("scan-root");
        let users = root.join("Users");
        let alice = users.join("Alice");
        let empty = users.join("Empty");
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10 * 86_400);
        let mut accumulators = NtfsStorageAccumulator::new(&root, 5);

        accumulators.record_file(12, &alice, 70, Some(now));
        accumulators.record_file(5, &root, 20, Some(now));
        accumulators.record_directory(12, 10, &alice);
        accumulators.record_directory(13, 10, &empty);
        accumulators.record_directory(10, 5, &users);

        let index = accumulators.into_storage_index(&root, now);
        let root_areas = index.areas_for(&root.to_string_lossy()).unwrap();
        let users_area = root_areas.iter().find(|area| area.name == "Users").unwrap();
        assert_eq!(users_area.size_bytes, 70);
        assert_eq!(users_area.file_count, 1);
        assert!(root_areas.iter().any(|area| {
            area.name == "Files at root" && area.size_bytes == 20 && area.file_count == 1
        }));

        let user_areas = index.areas_for(&users.to_string_lossy()).unwrap();
        assert!(
            user_areas.iter().any(|area| {
                area.name == "Alice" && area.size_bytes == 70 && area.file_count == 1
            })
        );
        assert!(
            user_areas.iter().any(|area| {
                area.name == "Empty" && area.size_bytes == 0 && area.file_count == 0
            })
        );
        let alice_areas = index.areas_for(&alice.to_string_lossy()).unwrap();
        assert_eq!(alice_areas.len(), 1);
        assert_eq!(alice_areas[0].name, "Files in this folder");
        assert_eq!(alice_areas[0].size_bytes, 70);
    }

    #[test]
    fn storage_index_subtracts_a_deleted_file_at_every_level() {
        let root = PathBuf::from("scan-root");
        let file = root.join("Users").join("Alice").join("report.txt");
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10 * 86_400);
        let mut accumulators = StorageAreaAccumulators::new();
        accumulators.insert(root.clone(), HashMap::new());
        record_storage_file(&root, &file, 70, Some(now), &mut accumulators);
        let mut index = StorageIndex::from_accumulators(&root, accumulators, now);

        index.remove_files(&[(file.to_string_lossy().to_string(), 70)]);

        let root_areas = index.areas_for(&root.to_string_lossy()).unwrap();
        assert_eq!(root_areas[0].size_bytes, 0);
        assert_eq!(root_areas[0].file_count, 0);
        let alice_areas = index
            .areas_for(&root.join("Users").join("Alice").to_string_lossy())
            .unwrap();
        assert!(alice_areas.is_empty());
    }

    fn temporary_large_file_root(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "luna-large-file-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("current time")
                .as_nanos()
        ))
    }

    fn indexed_large_file(root: &Path, name: &str, bytes: &[u8]) -> (LargeFileIndex, String) {
        fs::create_dir_all(root).expect("temporary scan root should be created");
        let path = root.join(name);
        fs::write(&path, bytes).expect("temporary large file should be written");
        let canonical_root = fs::canonicalize(root).expect("scan root should be canonicalized");
        let canonical_path = fs::canonicalize(&path).expect("large file should be canonicalized");
        let display_path = canonical_path.to_string_lossy().to_string();
        let record = LargeFile {
            name: name.to_string(),
            path: display_path.clone(),
            size_bytes: bytes.len() as u64,
            last_used_days: Some(12),
            modified_at: None,
        };
        (
            LargeFileIndex::from_scan(&canonical_root.to_string_lossy(), &[record]),
            display_path,
        )
    }

    #[test]
    fn large_file_index_deletes_only_a_current_scan_entry() {
        let root = temporary_large_file_root("delete");
        let (mut index, path) = indexed_large_file(&root, "archive.bin", b"large-file");

        let metadata = index.metadata_for(&path).unwrap();
        assert_eq!(metadata.relative_path, "archive.bin");
        assert!(
            !metadata
                .relative_path
                .contains(&root.to_string_lossy().to_string())
        );

        let result = index.delete_files(std::slice::from_ref(&path)).unwrap();

        assert_eq!(result.removed_files, 1);
        assert_eq!(result.removed_bytes, 10);
        assert!(!Path::new(&path).exists());
        assert!(index.delete_files(&[path]).is_err());
        fs::remove_dir_all(root).expect("temporary scan root should be removed");
    }

    #[test]
    fn large_file_index_requires_a_rescan_when_size_changes() {
        let root = temporary_large_file_root("changed");
        let (mut index, path) = indexed_large_file(&root, "video.bin", b"original");
        fs::write(&path, b"changed-size").expect("temporary large file should change");

        let result = index.delete_files(std::slice::from_ref(&path)).unwrap();

        assert_eq!(result.removed_files, 0);
        assert_eq!(result.failed.len(), 1);
        assert!(Path::new(&path).exists());
        fs::remove_dir_all(root).expect("temporary scan root should be removed");
    }

    #[cfg(windows)]
    #[test]
    fn canonical_windows_drive_root_matches_reported_volume() {
        let drive = list_scan_roots()
            .into_iter()
            .find(|root| root.total_bytes > 0)
            .expect("a Windows drive");
        let canonical_root = fs::canonicalize(&drive.path).expect("canonical drive root");

        let space = volume_space_for_root(&canonical_root).expect("matching Windows volume");

        assert_eq!(space.total_bytes, drive.total_bytes);
    }

    #[cfg(windows)]
    #[test]
    fn only_a_full_ntfs_volume_requests_scan_elevation() {
        assert!(!is_full_ntfs_volume_path(
            &env::temp_dir().to_string_lossy()
        ));

        let disks = Disks::new_with_refreshed_list();
        if let Some(disk) = disks.iter().find(|disk| {
            disk.file_system()
                .to_string_lossy()
                .eq_ignore_ascii_case("ntfs")
        }) {
            assert!(is_full_ntfs_volume_path(
                &disk.mount_point().to_string_lossy()
            ));
        }
    }
}
