use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[cfg(windows)]
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
#[cfg(windows)]
const FILE_ATTRIBUTE_PINNED: u32 = 0x0008_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_UNPINNED: u32 = 0x0010_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

#[derive(Debug, Clone, Default)]
pub(crate) struct CloudFilePolicy {
    one_drive_roots: Vec<PathBuf>,
}

impl CloudFilePolicy {
    pub(crate) fn from_environment() -> Self {
        let mut roots = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
            .into_iter()
            .filter_map(env::var_os)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();

        if let Some(profile) = env::var_os("USERPROFILE").map(PathBuf::from) {
            roots.push(profile.join("OneDrive"));
            if let Ok(entries) = fs::read_dir(&profile) {
                roots.extend(entries.flatten().filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                    name.starts_with("onedrive - ").then(|| entry.path())
                }));
            }
        }

        Self::from_roots(roots)
    }

    pub(crate) fn from_roots(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut seen = HashSet::new();
        let one_drive_roots = roots
            .into_iter()
            .filter(|root| !root.as_os_str().is_empty())
            .filter(|root| seen.insert(normalized_path_key(root)))
            .collect();
        Self { one_drive_roots }
    }

    pub(crate) fn is_one_drive_path(&self, path: &Path) -> bool {
        if path.components().any(|component| {
            let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
            name == "onedrive" || name.starts_with("onedrive - ")
        }) {
            return true;
        }

        let path = normalized_path_key(path);
        self.one_drive_roots.iter().any(|root| {
            let root = normalized_path_key(root);
            path == root
                || path
                    .strip_prefix(&root)
                    .is_some_and(|suffix| suffix.starts_with(path_separator()))
        })
    }

    pub(crate) fn content_access_error(
        &self,
        path: &Path,
        metadata: &fs::Metadata,
    ) -> Option<&'static str> {
        if self.is_one_drive_path(path) {
            Some("Luna keeps OneDrive files metadata-only and will not open their contents.")
        } else if is_online_only(metadata) {
            Some("Luna will not download or open an online-only cloud file.")
        } else {
            None
        }
    }
}

#[cfg(windows)]
pub(crate) fn local_size_bytes(metadata: &fs::Metadata) -> u64 {
    local_size_bytes_for_attributes(metadata.len(), metadata.file_attributes())
}

#[cfg(not(windows))]
pub(crate) fn local_size_bytes(metadata: &fs::Metadata) -> u64 {
    metadata.len()
}

#[cfg(windows)]
pub(crate) fn is_online_only(metadata: &fs::Metadata) -> bool {
    is_online_only_attributes(metadata.file_attributes())
}

#[cfg(not(windows))]
pub(crate) fn is_online_only(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
pub(crate) fn is_always_kept(metadata: &fs::Metadata) -> bool {
    is_always_kept_attributes(metadata.file_attributes())
}

#[cfg(not(windows))]
pub(crate) fn is_always_kept(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
pub(crate) fn is_online_only_attributes(attributes: u32) -> bool {
    attributes
        & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_UNPINNED | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
}

#[cfg(windows)]
pub(crate) fn is_always_kept_attributes(attributes: u32) -> bool {
    attributes & FILE_ATTRIBUTE_PINNED != 0
}

#[cfg(windows)]
pub(crate) fn local_size_bytes_for_attributes(logical_size: u64, attributes: u32) -> u64 {
    if is_online_only_attributes(attributes) {
        0
    } else {
        logical_size
    }
}

#[cfg(windows)]
fn normalized_path_key(path: &Path) -> String {
    let mut value = path.to_string_lossy().replace('/', "\\");
    if let Some(stripped) = value.strip_prefix("\\\\?\\UNC\\") {
        value = format!("\\\\{stripped}");
    } else if let Some(stripped) = value.strip_prefix("\\\\?\\") {
        value = stripped.to_string();
    }
    value.trim_end_matches('\\').to_ascii_lowercase()
}

#[cfg(not(windows))]
fn normalized_path_key(path: &Path) -> String {
    path.to_string_lossy().trim_end_matches('/').to_string()
}

#[cfg(windows)]
fn path_separator() -> char {
    '\\'
}

#[cfg(not(windows))]
fn path_separator() -> char {
    '/'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_drive_paths_are_matched_without_touching_the_filesystem() {
        let policy =
            CloudFilePolicy::from_roots([PathBuf::from(r"C:\Users\Alice\OneDrive - Personal")]);

        assert!(policy.is_one_drive_path(Path::new(
            r"\\?\c:\users\ALICE\ONEDRIVE - PERSONAL\Documents\online.raw"
        )));
        assert!(policy.is_one_drive_path(Path::new(
            r"D:\Relocated\OneDrive - Work\Documents\online.raw"
        )));
        assert!(!policy.is_one_drive_path(Path::new(r"C:\Users\Alice\Documents\local.raw")));
        assert!(!policy.is_one_drive_path(Path::new(r"C:\Users\Alice\OneDrive Backup\other.raw")));
    }

    #[cfg(windows)]
    #[test]
    fn recall_attributes_identify_files_without_local_contents() {
        assert!(is_online_only_attributes(FILE_ATTRIBUTE_OFFLINE));
        assert!(is_online_only_attributes(
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
        ));
        assert!(is_online_only_attributes(FILE_ATTRIBUTE_UNPINNED));
        assert!(!is_online_only_attributes(0x0000_0400));
        assert!(!is_online_only_attributes(FILE_ATTRIBUTE_PINNED));
        assert_eq!(
            local_size_bytes_for_attributes(4_294_967_296, FILE_ATTRIBUTE_UNPINNED),
            0
        );
        assert_eq!(
            local_size_bytes_for_attributes(4_294_967_296, FILE_ATTRIBUTE_PINNED),
            4_294_967_296
        );
        assert_eq!(local_size_bytes_for_attributes(512, 0), 512);
    }
}
