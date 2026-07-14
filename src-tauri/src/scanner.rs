use crate::models::{
    AgeBuckets, CleanupItem, CleanupResult, DeletedLargeFile, DuplicateFile, DuplicateGroup,
    LargeFile, LargeFileDeleteResult, ScanProgress, ScanResult, ScanRootInfo, StorageCategory,
};
use blake3::Hasher;
use chrono::{DateTime, Local, SecondsFormat};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet},
    env,
    fs::{self, File},
    io::{BufReader, Read},
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};
use sysinfo::Disks;
use walkdir::{DirEntry, WalkDir};

const LARGE_FILE_LIMIT: usize = 40;
const LARGE_FILE_DELETE_LIMIT: usize = LARGE_FILE_LIMIT;
const DUPLICATE_CANDIDATE_LIMIT: usize = 20_000;
const DUPLICATE_SIZE_GROUP_LIMIT: usize = 60;
const DUPLICATE_FILES_PER_GROUP_LIMIT: usize = 12;
const MIN_DUPLICATE_SIZE: u64 = 1_048_576;

#[derive(Debug, Clone)]
struct CandidateFile {
    path: PathBuf,
    activity: Option<SystemTime>,
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct StorageIndex {
    pub(crate) root: String,
    pub(crate) children: HashMap<String, Vec<StorageCategory>>,
}

impl StorageIndex {
    fn from_accumulators(
        root: &Path,
        accumulators: StorageAreaAccumulators,
        now: SystemTime,
    ) -> Self {
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
        let path = Path::new(&record.path);
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

pub fn scan_path<F>(requested_path: &str, mut on_progress: F) -> Result<ScanOutput, String>
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
    let mut total_bytes = 0_u64;
    let mut file_count = 0_u64;
    let mut folder_count = 0_u64;
    let mut storage_areas = StorageAreaAccumulators::new();
    storage_areas.insert(root.clone(), HashMap::new());
    let mut ages = AgeBuckets::default();
    let mut largest: BinaryHeap<Reverse<(u64, String, Option<SystemTime>)>> = BinaryHeap::new();
    let mut duplicate_candidates: HashMap<u64, Vec<CandidateFile>> = HashMap::new();
    let mut duplicate_candidate_count = 0_usize;
    let mut warnings = Vec::new();

    let walker = WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_descend(entry));

    for result in walker {
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                push_warning(
                    &mut warnings,
                    format!("Skipped an unreadable location: {error}"),
                );
                continue;
            }
        };

        if entry.depth() == 0 {
            continue;
        }

        if entry.file_type().is_dir() {
            folder_count = folder_count.saturating_add(1);
            record_storage_directory(&root, entry.path(), &mut storage_areas);
            continue;
        }

        if !entry.file_type().is_file() {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                push_warning(
                    &mut warnings,
                    format!("Skipped metadata for {}: {error}", entry.path().display()),
                );
                continue;
            }
        };

        let size = metadata.len();
        let activity = activity_time(&metadata);
        total_bytes = total_bytes.saturating_add(size);
        file_count = file_count.saturating_add(1);
        add_age_bytes(&mut ages, size, activity, now);
        record_storage_file(&root, entry.path(), size, activity, &mut storage_areas);

        let display_path = entry.path().to_string_lossy().to_string();
        largest.push(Reverse((size, display_path, activity)));
        if largest.len() > LARGE_FILE_LIMIT {
            largest.pop();
        }

        if size >= MIN_DUPLICATE_SIZE && duplicate_candidate_count < DUPLICATE_CANDIDATE_LIMIT {
            duplicate_candidates
                .entry(size)
                .or_default()
                .push(CandidateFile {
                    path: entry.path().to_path_buf(),
                    activity,
                });
            duplicate_candidate_count += 1;
        }

        if file_count.is_multiple_of(1_000) {
            on_progress(scan_progress(
                file_count,
                total_bytes,
                entry.path(),
                progress_volume_space,
            ));
        }
    }

    if duplicate_candidate_count == DUPLICATE_CANDIDATE_LIMIT {
        push_warning(
            &mut warnings,
            "Duplicate analysis reached its 20,000-file safety limit; the storage totals remain complete."
                .to_string(),
        );
    }

    let duplicate_groups = find_duplicate_groups(duplicate_candidates, now, &mut warnings);
    let cleanup_items = build_cleanup_items(&duplicate_groups, now, &mut warnings);

    let storage_index = StorageIndex::from_accumulators(&root, storage_areas, now);
    let mut categories = storage_index.areas_for(&root.to_string_lossy())?;
    categories.truncate(24);

    let mut large_files: Vec<LargeFile> = largest
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
        file_count,
        total_bytes,
        &root,
        progress_volume_space,
    ));

    let root_name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| root.to_string_lossy().to_string());

    let volume_space = volume_space_for_root(&root);

    Ok(ScanOutput {
        result: ScanResult {
            root: root.to_string_lossy().to_string(),
            root_name,
            total_bytes,
            drive_total_bytes: volume_space.map(|space| space.total_bytes),
            drive_used_bytes: volume_space.map(VolumeSpace::used_bytes),
            file_count,
            folder_count,
            categories,
            large_files,
            duplicate_groups,
            cleanup_items,
            age_buckets: ages,
            scanned_at: format_time(SystemTime::now()),
            duration_ms: started.elapsed().as_millis(),
            warnings,
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

    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
    !matches!(
        name.as_str(),
        "$recycle.bin" | "system volume information" | "recovery" | ".git" | "node_modules"
    )
}

fn record_storage_file(
    root: &Path,
    file: &Path,
    size: u64,
    activity: Option<SystemTime>,
    storage_areas: &mut StorageAreaAccumulators,
) {
    let mut child = file;
    while let Some(parent) = child.parent() {
        if !parent.starts_with(root) {
            break;
        }

        let is_direct_file = child == file;
        let area_path = if is_direct_file { parent } else { child };
        let name = if is_direct_file {
            if parent == root {
                "Files at root".to_string()
            } else {
                "Files in this folder".to_string()
            }
        } else {
            child
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| child.to_string_lossy().to_string())
        };

        let area = storage_areas
            .entry(parent.to_path_buf())
            .or_default()
            .entry(area_path.to_path_buf())
            .or_insert_with(|| CategoryAccumulator {
                name,
                path: area_path.to_path_buf(),
                can_drill_down: !is_direct_file,
                ..CategoryAccumulator::default()
            });
        area.size_bytes = area.size_bytes.saturating_add(size);
        area.file_count = area.file_count.saturating_add(1);
        area.newest_activity = newest_time(area.newest_activity, activity);

        if parent == root {
            break;
        }
        child = parent;
    }
}

fn record_storage_directory(
    root: &Path,
    directory: &Path,
    storage_areas: &mut StorageAreaAccumulators,
) {
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
        let mut hashes: HashMap<String, Vec<CandidateFile>> = HashMap::new();
        for candidate in files.into_iter().take(DUPLICATE_FILES_PER_GROUP_LIMIT) {
            match hash_file(&candidate.path) {
                Ok(hash) => hashes.entry(hash).or_default().push(candidate),
                Err(error) => push_warning(
                    warnings,
                    format!("Could not compare {}: {error}", candidate.path.display()),
                ),
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

    let browser_stats = combine_target_stats(&browser, warnings);
    let codex_stats = combine_target_stats(&codex, warnings);
    let temp_stats = combine_target_stats(&temporary, warnings);
    let old_download_stats = scan_old_downloads(now, warnings);
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
            browser.len(),
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
            codex.len(),
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
            temporary.len(),
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
            1,
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
    evidence_count: usize,
) -> CleanupItem {
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

fn combine_target_stats(targets: &[&CacheTarget], warnings: &mut Vec<String>) -> PathStats {
    let mut combined = PathStats::default();
    for target in targets {
        let stats = collect_path_stats(&target.path, warnings);
        combined.size_bytes = combined.size_bytes.saturating_add(stats.size_bytes);
        combined.file_count = combined.file_count.saturating_add(stats.file_count);
        combined.newest_activity = newest_time(combined.newest_activity, stats.newest_activity);
        combined.oldest_activity = oldest_time(combined.oldest_activity, stats.oldest_activity);
    }
    combined
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

fn scan_old_downloads(now: SystemTime, warnings: &mut Vec<String>) -> PathStats {
    let Some(downloads) = dirs::download_dir() else {
        return PathStats::default();
    };

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
}
