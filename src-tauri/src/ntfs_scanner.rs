use ntfs_reader::{
    api::{
        EPOCH_DIFFERENCE, NtfsAttributeType, NtfsFileNameHeader, NtfsFileNamespace,
        NtfsFileRecordHeader, ROOT_RECORD, SECTOR_SIZE,
    },
    attribute::NtfsAttribute,
    file::NtfsFile,
    mft::Mft,
    volume::Volume,
};
use std::{
    collections::HashMap,
    ffi::OsString,
    fs::OpenOptions,
    io::{self, Read, Seek, SeekFrom},
    mem::size_of,
    os::windows::{ffi::OsStringExt, fs::OpenOptionsExt},
    path::{Component, Path, PathBuf, Prefix},
    thread,
    time::{Duration, Instant, SystemTime},
};
use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_SEQUENTIAL_SCAN;

const RAW_VOLUME_ALIGNMENT: u64 = 4_096;
const MAX_DIRECT_READ_BYTES: usize = 16 * 1_024 * 1_024;
const MAX_FIXUP_WORKERS: usize = 8;
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

struct FastAlignedReader<R> {
    inner: R,
    alignment: u64,
    position: u64,
    buffer_position: u64,
    buffer_size: usize,
    buffer: Vec<u8>,
}

impl<R> FastAlignedReader<R>
where
    R: Read + Seek,
{
    fn new(inner: R, alignment: u64) -> io::Result<Self> {
        if !alignment.is_power_of_two() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "raw-volume alignment must be a power of two",
            ));
        }
        Ok(Self {
            inner,
            alignment,
            position: 0,
            buffer_position: 0,
            buffer_size: 0,
            buffer: Vec::with_capacity(alignment as usize),
        })
    }

    fn round_down(&self, value: u64) -> u64 {
        value / self.alignment * self.alignment
    }

    fn round_up(&self, value: u64) -> u64 {
        if value.is_multiple_of(self.alignment) {
            value
        } else {
            self.round_down(value) + self.alignment
        }
    }
}

impl<R> Read for FastAlignedReader<R>
where
    R: Read + Seek,
{
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }

        let aligned_position = self.round_down(self.position);
        let offset = (self.position - aligned_position) as usize;
        let alignment = self.alignment as usize;

        if offset == 0 && output.len() >= alignment {
            let direct_length = output.len().min(MAX_DIRECT_READ_BYTES) / alignment * alignment;
            self.inner.seek(SeekFrom::Start(self.position))?;
            self.inner.read_exact(&mut output[..direct_length])?;
            self.position += direct_length as u64;
            self.buffer_size = 0;
            return Ok(direct_length);
        }

        let copy_length = output.len().min(alignment - offset);
        let required = self.round_up((offset + copy_length) as u64) as usize;
        if aligned_position != self.buffer_position || required > self.buffer_size {
            self.inner.seek(SeekFrom::Start(aligned_position))?;
            self.buffer.resize(required, 0);
            self.inner.read_exact(&mut self.buffer)?;
            self.buffer_position = aligned_position;
            self.buffer_size = required;
        }

        output[..copy_length].copy_from_slice(&self.buffer[offset..offset + copy_length]);
        self.position += copy_length as u64;
        Ok(copy_length)
    }
}

impl<R> Seek for FastAlignedReader<R>
where
    R: Read + Seek,
{
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let position = match position {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) if offset >= 0 => self.position.checked_add(offset as u64),
            SeekFrom::Current(offset) => self.position.checked_sub(offset.unsigned_abs()),
            SeekFrom::End(_) => None,
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid raw-volume seek"))?;
        self.position = position;
        Ok(position)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NtfsScanSummary {
    pub(crate) unresolved_records: u64,
    pub(crate) catalogue_read_ms: u128,
    pub(crate) record_fixup_ms: u128,
    pub(crate) record_parse_ms: u128,
    pub(crate) used_compatibility_reader: bool,
}

#[derive(Debug, Clone, Copy)]
struct NtfsLoadTimings {
    catalogue_read_ms: u128,
    record_fixup_ms: u128,
    used_compatibility_reader: bool,
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
    let (mft, load_timings) = load_mft(volume)?;
    let parse_started = Instant::now();

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

    Ok(Some(NtfsScanSummary {
        unresolved_records,
        catalogue_read_ms: load_timings.catalogue_read_ms,
        record_fixup_ms: load_timings.record_fixup_ms,
        record_parse_ms: parse_started.elapsed().as_millis(),
        used_compatibility_reader: load_timings.used_compatibility_reader,
    }))
}

fn load_mft(volume: Volume) -> Result<(Mft, NtfsLoadTimings), String> {
    match load_mft_fast(volume.clone()) {
        Ok(result) => Ok(result),
        Err(fast_error) => {
            let started = Instant::now();
            let mft = Mft::new(volume).map_err(|compatibility_error| {
                format!(
                    "Luna could not parse the NTFS master catalogue with either reader (wide-read error: {fast_error}; compatibility error: {compatibility_error})"
                )
            })?;
            Ok((
                mft,
                NtfsLoadTimings {
                    catalogue_read_ms: started.elapsed().as_millis(),
                    record_fixup_ms: 0,
                    used_compatibility_reader: true,
                },
            ))
        }
    }
}

fn load_mft_fast(volume: Volume) -> Result<(Mft, NtfsLoadTimings), String> {
    let record_size = usize::try_from(volume.file_record_size)
        .ok()
        .filter(|size| *size >= size_of::<NtfsFileRecordHeader>())
        .ok_or_else(|| "The NTFS catalogue reported an invalid file-record size.".to_string())?;

    let read_started = Instant::now();
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN)
        .open(&volume.path)
        .map_err(|error| format!("Luna could not open the raw NTFS volume ({error})"))?;
    let mut reader = FastAlignedReader::new(file, RAW_VOLUME_ALIGNMENT)
        .map_err(|error| format!("Luna could not prepare the raw NTFS reader ({error})"))?;
    let mft_record = Mft::get_record_fs(&mut reader, volume.file_record_size, volume.mft_position)
        .map_err(|error| format!("Luna could not read the NTFS catalogue record ({error})"))?;
    let mut data = Mft::read_data_fs(&volume, &mut reader, &mft_record, NtfsAttributeType::Data)
        .map_err(|error| format!("Luna could not bulk-read the NTFS catalogue ({error})"))?
        .ok_or_else(|| "The NTFS catalogue did not contain its record stream.".to_string())?;
    let bitmap = Mft::read_data_fs(&volume, &mut reader, &mft_record, NtfsAttributeType::Bitmap)
        .map_err(|error| format!("Luna could not read the NTFS catalogue bitmap ({error})"))?
        .ok_or_else(|| {
            "The NTFS catalogue did not contain its active-record bitmap.".to_string()
        })?;
    let catalogue_read_ms = read_started.elapsed().as_millis();

    let max_record = (data.len() / record_size) as u64;
    let fixup_started = Instant::now();
    fixup_active_records(&mut data, &bitmap, record_size)?;
    let record_fixup_ms = fixup_started.elapsed().as_millis();

    Ok((
        Mft {
            volume,
            data,
            bitmap,
            max_record,
        },
        NtfsLoadTimings {
            catalogue_read_ms,
            record_fixup_ms,
            used_compatibility_reader: false,
        },
    ))
}

fn fixup_active_records(data: &mut [u8], bitmap: &[u8], record_size: usize) -> Result<(), String> {
    if record_size < size_of::<NtfsFileRecordHeader>() {
        return Err("The NTFS catalogue reported an invalid file-record size.".to_string());
    }
    let record_count = data.len() / record_size;
    if record_count == 0 {
        return Ok(());
    }

    let workers = thread::available_parallelism()
        .map_or(1, usize::from)
        .min(MAX_FIXUP_WORKERS)
        .min(record_count);
    let records_per_worker = record_count.div_ceil(workers);
    let bytes_per_worker = records_per_worker.saturating_mul(record_size);

    let result = thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for (chunk_index, chunk) in data[..record_count * record_size]
            .chunks_mut(bytes_per_worker)
            .enumerate()
        {
            handles.push(scope.spawn(move || {
                let first_record = chunk_index * records_per_worker;
                for (offset, record) in chunk.chunks_exact_mut(record_size).enumerate() {
                    let record_number = first_record + offset;
                    if record_is_active(bitmap, record_number)
                        && fixup_record(record_number as u64, record).is_err()
                    {
                        return Err(record_number as u64);
                    }
                }
                Ok(())
            }));
        }

        for handle in handles {
            let worker_result = handle.join().map_err(|_| u64::MAX)?;
            worker_result?;
        }
        Ok::<(), u64>(())
    });

    match result {
        Ok(()) => Ok(()),
        Err(u64::MAX) => Err("An NTFS catalogue repair worker stopped unexpectedly.".to_string()),
        Err(record_number) => Err(format!(
            "The NTFS catalogue contained a corrupt active record ({record_number})."
        )),
    }
}

fn record_is_active(bitmap: &[u8], record_number: usize) -> bool {
    bitmap
        .get(record_number / 8)
        .is_some_and(|byte| byte & (1 << (record_number % 8)) != 0)
}

fn fixup_record(_record_number: u64, data: &mut [u8]) -> Result<(), ()> {
    if data.len() < size_of::<NtfsFileRecordHeader>() {
        return Err(());
    }
    // SAFETY: the length check guarantees a complete header, and the record
    // stream does not promise native alignment.
    let header = unsafe {
        data.as_ptr()
            .cast::<NtfsFileRecordHeader>()
            .read_unaligned()
    };
    let update_sequence_start = header.update_sequence_offset as usize;
    if update_sequence_start + 2 > data.len() {
        return Err(());
    }
    let replacement_start = update_sequence_start + 2;
    let replacement_end = update_sequence_start
        .saturating_add((header.update_sequence_length as usize).saturating_mul(2));
    if replacement_end > data.len() {
        return Err(());
    }

    let update_sequence = [data[update_sequence_start], data[update_sequence_start + 1]];
    let mut sector_end = SECTOR_SIZE - 2;
    for replacement in (replacement_start..replacement_end).step_by(2) {
        if sector_end + 2 > data.len() {
            break;
        }
        if data[sector_end..sector_end + 2] != update_sequence {
            return Err(());
        }
        let replacement_bytes = [data[replacement], data[replacement + 1]];
        data[sector_end..sector_end + 2].copy_from_slice(&replacement_bytes);
        sector_end += SECTOR_SIZE;
    }
    Ok(())
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
    use std::{
        io::Cursor,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    struct CountingReader {
        cursor: Cursor<Vec<u8>>,
        reads: Arc<AtomicUsize>,
    }

    impl Read for CountingReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.cursor.read(output)
        }
    }

    impl Seek for CountingReader {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.cursor.seek(position)
        }
    }

    fn protected_record() -> Vec<u8> {
        let mut record = vec![0_u8; 1_024];
        record[4..6].copy_from_slice(&48_u16.to_le_bytes());
        record[6..8].copy_from_slice(&3_u16.to_le_bytes());
        record[48..54].copy_from_slice(&[0xAA, 0xBB, 1, 2, 3, 4]);
        record[510..512].copy_from_slice(&[0xAA, 0xBB]);
        record[1_022..1_024].copy_from_slice(&[0xAA, 0xBB]);
        record
    }

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

    #[test]
    fn aligned_reader_uses_one_large_read_for_aligned_catalogue_data() {
        let input: Vec<u8> = (0..131_072).map(|index| index as u8).collect();
        let reads = Arc::new(AtomicUsize::new(0));
        let counting_reader = CountingReader {
            cursor: Cursor::new(input.clone()),
            reads: Arc::clone(&reads),
        };
        let mut reader = FastAlignedReader::new(counting_reader, RAW_VOLUME_ALIGNMENT).unwrap();
        let mut output = vec![0_u8; 65_536];

        reader.read_exact(&mut output).unwrap();

        assert_eq!(output, input[..output.len()]);
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn aligned_reader_preserves_unaligned_seek_and_tail_reads() {
        let input: Vec<u8> = (0..32_768).map(|index| index as u8).collect();
        let reads = Arc::new(AtomicUsize::new(0));
        let counting_reader = CountingReader {
            cursor: Cursor::new(input.clone()),
            reads,
        };
        let mut reader = FastAlignedReader::new(counting_reader, RAW_VOLUME_ALIGNMENT).unwrap();
        let mut output = vec![0_u8; 10_000];

        reader.seek(SeekFrom::Start(123)).unwrap();
        reader.read_exact(&mut output).unwrap();

        assert_eq!(output, input[123..10_123]);
    }

    #[test]
    fn parallel_fixup_repairs_only_bitmap_active_records() {
        let inactive = protected_record();
        let active = protected_record();
        let mut data = [inactive, active].concat();

        fixup_active_records(&mut data, &[0b0000_0010], 1_024).unwrap();

        assert_eq!(&data[510..512], &[0xAA, 0xBB]);
        assert_eq!(&data[1_022..1_024], &[0xAA, 0xBB]);
        assert_eq!(&data[1_024 + 510..1_024 + 512], &[1, 2]);
        assert_eq!(&data[1_024 + 1_022..1_024 + 1_024], &[3, 4]);
    }

    #[test]
    fn parallel_fixup_rejects_a_corrupt_active_record() {
        let mut record = protected_record();
        record[510] = 0;

        let error = fixup_active_records(&mut record, &[0b0000_0001], 1_024).unwrap_err();

        assert!(error.contains("corrupt active record (0)"));
    }
}
