use ntfs_reader::{
    api::{EPOCH_DIFFERENCE, NtfsAttributeType, NtfsFileName, NtfsFileNamespace, ROOT_RECORD},
    file::NtfsFile,
    mft::Mft,
    volume::Volume,
};
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf, Prefix},
    time::{Duration, SystemTime},
};

const MAX_PATH_DEPTH: usize = 1_024;

#[derive(Debug)]
pub(crate) struct NtfsEntry {
    pub(crate) path: PathBuf,
    pub(crate) is_directory: bool,
    pub(crate) logical_size: u64,
    pub(crate) activity: Option<SystemTime>,
    pub(crate) file_attributes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NtfsScanSummary {
    pub(crate) unresolved_records: u64,
}

/// Attempts a bulk `$MFT` inventory for a full Windows drive root.
///
/// `Ok(None)` means that the requested path is not a full drive root. Errors are
/// recoverable: the caller should retain the regular Windows directory walker as
/// a fallback for non-NTFS volumes, non-elevated processes, and malformed data.
pub(crate) fn scan_volume<F>(
    root: &Path,
    mut on_entry: F,
) -> Result<Option<NtfsScanSummary>, String>
where
    F: FnMut(NtfsEntry),
{
    let Some(device_path) = volume_device_path(root) else {
        return Ok(None);
    };

    let volume = Volume::new(&device_path).map_err(|error| {
        format!("Windows did not make the NTFS master catalogue available ({error})")
    })?;
    let mft = Mft::new(volume)
        .map_err(|error| format!("Luna could not parse the NTFS master catalogue ({error})"))?;

    let mut directory_paths = HashMap::from([(ROOT_RECORD, root.to_path_buf())]);
    let mut unresolved_records = 0_u64;

    for file in mft.files() {
        let Some((file_name, metadata)) = entry_data_for(&file) else {
            unresolved_records = unresolved_records.saturating_add(1);
            continue;
        };
        let Some(path) = resolve_path(&mft, &file, file_name, &mut directory_paths) else {
            unresolved_records = unresolved_records.saturating_add(1);
            continue;
        };

        on_entry(NtfsEntry {
            path,
            is_directory: file.is_directory(),
            logical_size: metadata.logical_size,
            activity: metadata.activity,
            file_attributes: metadata.file_attributes,
        });
    }

    Ok(Some(NtfsScanSummary { unresolved_records }))
}

fn volume_device_path(root: &Path) -> Option<PathBuf> {
    let mut components = root.components();
    let letter = match components.next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
            _ => return None,
        },
        _ => return None,
    };
    if components.next() != Some(Component::RootDir) || components.next().is_some() {
        return None;
    }

    Some(PathBuf::from(format!(
        r"\\.\{}:",
        char::from(letter).to_ascii_uppercase()
    )))
}

pub(crate) fn is_volume_root(path: &Path) -> bool {
    volume_device_path(path).is_some()
}

#[derive(Debug, Default)]
struct NtfsMetadata {
    logical_size: u64,
    activity: Option<SystemTime>,
    file_attributes: u32,
}

fn entry_data_for(file: &NtfsFile<'_>) -> Option<(NtfsFileName, NtfsMetadata)> {
    let mut selected_name = None;
    let mut selected_priority = 0_u8;
    let mut activity = None;
    let mut standard_attributes = None;
    let mut data_size = None;

    file.attributes(|attribute| {
        let type_id = attribute.header.type_id;
        if type_id == NtfsAttributeType::FileName as u32 {
            let Some(name) = attribute.as_name() else {
                return;
            };
            let priority = file_name_priority(name.header.namespace);
            if priority > selected_priority {
                selected_name = Some(name);
                selected_priority = priority;
            }
            return;
        }

        if type_id == NtfsAttributeType::StandardInformation as u32 {
            if let Some(info) = attribute.as_standard_info() {
                activity = newest_ntfs_time(info.access_time, info.modification_time);
                standard_attributes = Some(info.file_attributes);
            }
            return;
        }

        if type_id != NtfsAttributeType::Data as u32 {
            return;
        }

        // Named $DATA attributes are alternate data streams, not the logical
        // size reported for the file by Windows.
        let name_length = attribute.header.name_length;
        if name_length != 0 {
            return;
        }

        if let Some(header) = attribute.resident_header() {
            data_size = Some(header.value_length as u64);
        } else if let Some(header) = attribute.nonresident_header() {
            // Only the first extent carries the full stream size.
            let lowest_vcn = header.lowest_vcn;
            if lowest_vcn == 0 {
                data_size = Some(header.data_size);
            }
        }
    });

    let file_name = selected_name?;
    Some((
        file_name,
        NtfsMetadata {
            logical_size: data_size.unwrap_or(file_name.header.real_size),
            activity,
            file_attributes: standard_attributes.unwrap_or(file_name.header.file_attributes),
        },
    ))
}

fn preferred_file_name(file: &NtfsFile<'_>) -> Option<NtfsFileName> {
    let mut selected = None;
    let mut selected_priority = 0_u8;

    file.attributes(|attribute| {
        let type_id = attribute.header.type_id;
        if type_id != NtfsAttributeType::FileName as u32 {
            return;
        }
        let Some(name) = attribute.as_name() else {
            return;
        };
        let priority = file_name_priority(name.header.namespace);
        if priority > selected_priority {
            selected = Some(name);
            selected_priority = priority;
        }
    });

    selected
}

fn file_name_priority(namespace: u8) -> u8 {
    match namespace {
        value if value == NtfsFileNamespace::Win32 as u8 => 3,
        value if value == NtfsFileNamespace::Win32AndDos as u8 => 3,
        value if value == NtfsFileNamespace::Posix as u8 => 2,
        value if value == NtfsFileNamespace::Dos as u8 => 1,
        _ => 0,
    }
}

fn resolve_path(
    mft: &Mft,
    file: &NtfsFile<'_>,
    file_name: NtfsFileName,
    directory_paths: &mut HashMap<u64, PathBuf>,
) -> Option<PathBuf> {
    let parent = resolve_directory_path(mft, file_name.parent(), directory_paths)?;
    let path = parent.join(file_name.to_string());
    if file.is_directory() {
        directory_paths.insert(file.number(), path.clone());
    }
    Some(path)
}

fn resolve_directory_path(
    mft: &Mft,
    mut record_number: u64,
    directory_paths: &mut HashMap<u64, PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = directory_paths.get(&record_number) {
        return Some(path.clone());
    }

    let mut chain = Vec::new();
    for _ in 0..MAX_PATH_DEPTH {
        if let Some(base) = directory_paths.get(&record_number).cloned() {
            let mut path = base;
            for (number, name) in chain.into_iter().rev() {
                path.push(name);
                directory_paths.insert(number, path.clone());
            }
            return Some(path);
        }

        let directory = mft.get_record(record_number)?;
        if !directory.is_used() || !directory.is_directory() {
            return None;
        }
        let name = preferred_file_name(&directory)?;
        let parent = name.parent();
        if parent == record_number {
            return None;
        }
        chain.push((record_number, name.to_string()));
        record_number = parent;
    }

    None
}

fn newest_ntfs_time(left: u64, right: u64) -> Option<SystemTime> {
    ntfs_time_to_system_time(left.max(right))
}

fn ntfs_time_to_system_time(value: u64) -> Option<SystemTime> {
    if value == 0 {
        return None;
    }

    if value >= EPOCH_DIFFERENCE {
        let intervals = value - EPOCH_DIFFERENCE;
        SystemTime::UNIX_EPOCH.checked_add(duration_from_ntfs_intervals(intervals))
    } else {
        let intervals = EPOCH_DIFFERENCE - value;
        SystemTime::UNIX_EPOCH.checked_sub(duration_from_ntfs_intervals(intervals))
    }
}

fn duration_from_ntfs_intervals(intervals: u64) -> Duration {
    Duration::new(
        intervals / 10_000_000,
        ((intervals % 10_000_000) * 100) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_volume_path_is_only_created_for_a_drive_root() {
        assert_eq!(
            volume_device_path(Path::new(r"C:\")),
            Some(PathBuf::from(r"\\.\C:"))
        );
        assert_eq!(
            volume_device_path(Path::new(r"\\?\d:\")),
            Some(PathBuf::from(r"\\.\D:"))
        );
        assert_eq!(volume_device_path(Path::new(r"C:\Users")), None);
        assert_eq!(volume_device_path(Path::new(r"\\server\share\")), None);
    }

    #[test]
    fn ntfs_epoch_and_unix_epoch_are_converted_without_overflow() {
        assert_eq!(
            ntfs_time_to_system_time(EPOCH_DIFFERENCE),
            Some(SystemTime::UNIX_EPOCH)
        );
        assert_eq!(
            ntfs_time_to_system_time(EPOCH_DIFFERENCE + 10_000_000),
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1))
        );
        assert_eq!(ntfs_time_to_system_time(0), None);
    }
}
