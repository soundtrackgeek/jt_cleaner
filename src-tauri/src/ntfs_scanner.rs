use ntfs_reader::{
    api::{
        EPOCH_DIFFERENCE, NtfsAttributeType, NtfsFileNameHeader, NtfsFileNamespace, ROOT_RECORD,
    },
    attribute::NtfsAttribute,
    file::NtfsFile,
    mft::Mft,
    volume::Volume,
};
use std::{
    collections::HashMap,
    ffi::OsString,
    mem::size_of,
    os::windows::ffi::OsStringExt,
    path::{Component, Path, PathBuf, Prefix},
    time::{Duration, SystemTime},
};

const MAX_PATH_DEPTH: usize = 1_024;
const MAX_DIRECTORY_CAPACITY_HINT: usize = 2_000_000;
pub(crate) const NTFS_ROOT_RECORD: u64 = ROOT_RECORD;

#[derive(Clone, Copy)]
struct NtfsName<'a> {
    header: NtfsFileNameHeader,
    utf16_bytes: &'a [u8],
}

impl NtfsName<'_> {
    fn parent(self) -> u64 {
        self.header.parent_directory_reference & 0x0000_FFFF_FFFF_FFFF
    }

    fn to_os_string(self) -> OsString {
        let mut wide = Vec::with_capacity(self.utf16_bytes.len() / 2);
        wide.extend(
            self.utf16_bytes
                .chunks_exact(2)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]])),
        );
        OsString::from_wide(&wide)
    }
}

pub(crate) struct NtfsEntry<'a> {
    pub(crate) directory_capacity_hint: usize,
    pub(crate) record_number: u64,
    pub(crate) parent_record_number: u64,
    pub(crate) parent_path: &'a Path,
    pub(crate) directory_path: Option<&'a Path>,
    pub(crate) is_directory: bool,
    pub(crate) logical_size: u64,
    pub(crate) activity: Option<SystemTime>,
    pub(crate) file_attributes: u32,
    file_name: NtfsName<'a>,
}

impl NtfsEntry<'_> {
    pub(crate) fn materialize_path(&self) -> PathBuf {
        self.directory_path
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.parent_path.join(self.file_name.to_os_string()))
    }
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
    F: for<'a> FnMut(NtfsEntry<'a>),
{
    let Some(device_path) = volume_device_path(root) else {
        return Ok(None);
    };

    let volume = Volume::new(&device_path).map_err(|error| {
        format!("Windows did not make the NTFS master catalogue available ({error})")
    })?;
    let mft = Mft::new(volume)
        .map_err(|error| format!("Luna could not parse the NTFS master catalogue ({error})"))?;

    let directory_capacity_hint = directory_capacity_hint(mft.max_record);
    let mut directory_paths = HashMap::with_capacity(directory_capacity_hint);
    directory_paths.insert(ROOT_RECORD, root.to_path_buf());
    let mut unresolved_records = 0_u64;

    for file in mft.files() {
        let Some((file_name, metadata)) = entry_data_for(&file) else {
            unresolved_records = unresolved_records.saturating_add(1);
            continue;
        };
        let parent_record_number = file_name.parent();
        if !ensure_directory_path(&mft, parent_record_number, &mut directory_paths) {
            unresolved_records = unresolved_records.saturating_add(1);
            continue;
        }

        let record_number = file.number();
        let is_directory = file.is_directory();
        if is_directory && record_number != ROOT_RECORD {
            let path = directory_paths
                .get(&parent_record_number)
                .expect("resolved NTFS parent path")
                .join(file_name.to_os_string());
            directory_paths.insert(record_number, path);
        }

        let parent_path = directory_paths
            .get(&parent_record_number)
            .expect("resolved NTFS parent path");
        let directory_path = is_directory
            .then(|| directory_paths.get(&record_number))
            .flatten()
            .map(PathBuf::as_path);

        on_entry(NtfsEntry {
            directory_capacity_hint,
            record_number,
            parent_record_number,
            parent_path,
            directory_path,
            is_directory,
            logical_size: metadata.logical_size,
            activity: metadata.activity,
            file_attributes: metadata.file_attributes,
            file_name,
        });
    }

    Ok(Some(NtfsScanSummary { unresolved_records }))
}

fn directory_capacity_hint(record_count: u64) -> usize {
    usize::try_from(record_count / 4)
        .unwrap_or(MAX_DIRECTORY_CAPACITY_HINT)
        .clamp(1_024, MAX_DIRECTORY_CAPACITY_HINT)
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

fn entry_data_for<'a>(file: &NtfsFile<'a>) -> Option<(NtfsName<'a>, NtfsMetadata)> {
    let mut selected_name = None;
    let mut selected_priority = 0_u8;
    let mut activity = None;
    let mut standard_attributes = None;
    let mut data_size = None;

    visit_attributes(file, |attribute| {
        let type_id = attribute.header.type_id;
        if type_id == NtfsAttributeType::FileName as u32 {
            let Some(name) = name_from_attribute(attribute) else {
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
    let real_size = file_name.header.real_size;
    let file_attributes = file_name.header.file_attributes;
    Some((
        file_name,
        NtfsMetadata {
            logical_size: data_size.unwrap_or(real_size),
            activity,
            file_attributes: standard_attributes.unwrap_or(file_attributes),
        },
    ))
}

fn preferred_file_name<'a>(file: &NtfsFile<'a>) -> Option<NtfsName<'a>> {
    let mut selected = None;
    let mut selected_priority = 0_u8;

    visit_attributes(file, |attribute| {
        let type_id = attribute.header.type_id;
        if type_id != NtfsAttributeType::FileName as u32 {
            return;
        }
        let Some(name) = name_from_attribute(attribute) else {
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

fn visit_attributes<'a, F>(file: &NtfsFile<'a>, mut visit: F)
where
    F: FnMut(&NtfsAttribute<'a>),
{
    let data: &'a [u8] = file.data;
    let mut offset = file.header.attributes_offset as usize;
    let used = usize::min(file.header.used_size as usize, data.len());

    while offset < used {
        let Some(attribute) = NtfsAttribute::new(&data[offset..used]) else {
            break;
        };
        if attribute.header.type_id == NtfsAttributeType::End as u32 {
            break;
        }
        visit(&attribute);

        let length = attribute.len();
        if length == 0 {
            break;
        }
        let Some(next) = offset.checked_add(length).filter(|next| *next <= used) else {
            break;
        };
        offset = next;
    }
}

fn name_from_attribute<'a>(attribute: &NtfsAttribute<'a>) -> Option<NtfsName<'a>> {
    let value = attribute.get_resident()?;
    name_from_resident_value(value)
}

fn name_from_resident_value(value: &[u8]) -> Option<NtfsName<'_>> {
    let header_size = size_of::<NtfsFileNameHeader>();
    if value.len() < header_size {
        return None;
    }

    // SAFETY: the length check above guarantees a complete header, and
    // `read_unaligned` does not require the resident value to be aligned.
    let header = unsafe { value.as_ptr().cast::<NtfsFileNameHeader>().read_unaligned() };
    let name_bytes = usize::from(header.name_length).checked_mul(2)?;
    let end = header_size.checked_add(name_bytes)?;
    if end > value.len() {
        return None;
    }

    Some(NtfsName {
        header,
        utf16_bytes: &value[header_size..end],
    })
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

fn ensure_directory_path(
    mft: &Mft,
    mut record_number: u64,
    directory_paths: &mut HashMap<u64, PathBuf>,
) -> bool {
    if directory_paths.contains_key(&record_number) {
        return true;
    }

    let mut chain = Vec::new();
    for _ in 0..MAX_PATH_DEPTH {
        if let Some(base) = directory_paths.get(&record_number).cloned() {
            let mut path = base;
            for (number, name) in chain.into_iter().rev() {
                path.push(name);
                directory_paths.insert(number, path.clone());
            }
            return true;
        }

        let Some(directory) = mft.get_record(record_number) else {
            return false;
        };
        if !directory.is_used() || !directory.is_directory() {
            return false;
        }
        let Some(name) = preferred_file_name(&directory) else {
            return false;
        };
        let parent = name.parent();
        if parent == record_number {
            return false;
        }
        chain.push((record_number, name.to_os_string()));
        record_number = parent;
    }

    false
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

    #[test]
    fn compact_ntfs_name_borrows_and_decodes_only_the_record_bytes() {
        let wide: Vec<u16> = "Résumé.txt".encode_utf16().collect();
        let mut value = vec![0_u8; size_of::<NtfsFileNameHeader>() + wide.len() * 2];
        value[..8].copy_from_slice(&42_u64.to_le_bytes());
        value[64] = wide.len() as u8;
        value[65] = NtfsFileNamespace::Win32 as u8;
        for (index, character) in wide.iter().enumerate() {
            let offset = size_of::<NtfsFileNameHeader>() + index * 2;
            value[offset..offset + 2].copy_from_slice(&character.to_le_bytes());
        }

        let name = name_from_resident_value(&value).expect("valid NTFS file name");

        assert_eq!(name.parent(), 42);
        assert_eq!(name.to_os_string(), OsString::from("Résumé.txt"));
    }
}
