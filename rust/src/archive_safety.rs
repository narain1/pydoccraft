//! Bounded ZIP directory preflight before upstream readers allocate indexes.
use std::{
    cell::Cell,
    collections::HashSet,
    io::{Cursor, Read},
    rc::Rc,
};

use zip_guard::{
    read::{ZipFile, ZipFileEntry},
    result::{ZipError, ZipResult},
    unstable::{
        format::blocks::Zip32CentralDirectoryEnd,
        stream::{ZipStreamReader, ZipStreamVisitor},
    },
};

use crate::{limits, EngineError};

fn malformed(message: impl std::fmt::Display) -> EngineError {
    EngineError::UnsafeArchive(format!("unsafe ZIP directory: {message}"))
}

struct DirectoryVisitor {
    count: usize,
    total: u64,
    names: HashSet<String>,
    raw_names: HashSet<Vec<u8>>,
    failure: Option<EngineError>,
}

impl ZipStreamVisitor for DirectoryVisitor {
    fn visit_file<R: Read>(&mut self, _: &mut ZipFile<'_, R>) -> ZipResult<()> {
        Err(ZipError::InvalidArchive(
            "unexpected local ZIP header".into(),
        ))
    }

    fn visit_additional_metadata(&mut self, entry: &ZipFileEntry<'_>) -> ZipResult<()> {
        let checked = (|| {
            self.count = self
                .count
                .checked_add(1)
                .ok_or_else(|| malformed("member count overflow"))?;
            limits::validate_member_count(self.count)?;
            let name = entry.name().map_err(malformed)?.replace('\\', "/");
            if !self.raw_names.insert(entry.name_raw().to_vec()) || !self.names.insert(name.clone())
            {
                return Err(malformed("duplicate member name"));
            }
            limits::validate_member(&name, entry.size(), entry.compressed_size())?;
            self.total = limits::add_uncompressed_size(self.total, entry.size())?;
            Ok(())
        })();
        if let Err(error) = checked {
            self.failure = Some(error);
            return Err(ZipError::InvalidArchive("ZIP safety limit".into()));
        }
        Ok(())
    }
}

struct CountedReader<'a> {
    inner: Cursor<&'a [u8]>,
    read: Rc<Cell<u64>>,
}

impl Read for CountedReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(bytes)?;
        self.read.set(self.read.get().saturating_add(count as u64));
        Ok(count)
    }
}

/// Bound raw directory records before name-keyed archive indexes are built.
pub(crate) fn validate_directory(data: &[u8]) -> Result<DirectoryInfo, EngineError> {
    limits::validate_input_size(data.len())?;
    const SIGNATURE: &[u8; 4] = b"PK\x05\x06";
    const MIN_END: usize = 22;
    const COMMENT_LIMIT: usize = u16::MAX as usize;
    let start = data.len().saturating_sub(MIN_END + COMMENT_LIMIT);
    let zip64_locator =
        zip_guard::unstable::format::magic::Magic::ZIP64_CENTRAL_DIRECTORY_END_LOCATOR_SIGNATURE
            .to_le_bytes();
    let mut selected = None;
    let mut candidates = 0usize;
    // Every candidate is bounded because upstream readers may fall back to an
    // earlier parseable end record if the last one is inconsistent.
    for (index, bytes) in data[start..].windows(SIGNATURE.len()).enumerate() {
        if bytes != SIGNATURE {
            continue;
        }
        candidates += 1;
        if candidates > 64 {
            return Err(EngineError::ResourceLimit(
                "ZIP has too many end-record candidates".into(),
            ));
        }
        let position = start + index;
        let mut candidate = Cursor::new(&data[position..]);
        let Ok(end) = Zip32CentralDirectoryEnd::parse(&mut candidate) else {
            continue;
        };
        if end.may_be_zip64()
            || position
                .checked_sub(20)
                .and_then(|offset| data.get(offset..offset + 4))
                == Some(zip64_locator.as_slice())
        {
            return Err(EngineError::Unsupported(
                "ZIP64 archives are not supported by bounded preflight".into(),
            ));
        }
        limits::validate_member_count(usize::from(end.number_of_files))?;
        limits::validate_member_count(usize::from(end.number_of_files_on_this_disk))?;
        if candidate.position() == (data.len() - position) as u64 {
            if selected.is_some() {
                return Err(malformed("ambiguous end records"));
            }
            selected = Some((position, end));
        }
    }
    let (end_pos, end) = selected
        .ok_or_else(|| EngineError::Malformed("invalid ZIP: missing complete end record".into()))?;
    if end.may_be_zip64()
        || end_pos
            .checked_sub(20)
            .and_then(|position| data.get(position..position + 4))
            == Some(zip64_locator.as_slice())
    {
        return Err(EngineError::Unsupported(
            "ZIP64 archives are not supported by bounded preflight".into(),
        ));
    }
    if end.disk_number != 0
        || end.disk_with_central_directory != 0
        || end.number_of_files != end.number_of_files_on_this_disk
    {
        return Err(EngineError::Unsupported(
            "multi-disk ZIP archives are not supported".into(),
        ));
    }
    limits::validate_member_count(usize::from(end.number_of_files))?;
    let directory_start = end.central_directory_offset as usize;
    let directory_size = end.central_directory_size as usize;
    if directory_start.checked_add(directory_size) != Some(end_pos) {
        return Err(malformed("directory offset and size disagree"));
    }
    let read = Rc::new(Cell::new(0));
    let reader = CountedReader {
        inner: Cursor::new(&data[directory_start..]),
        read: read.clone(),
    };
    let mut visitor = DirectoryVisitor {
        count: 0,
        total: 0,
        names: HashSet::new(),
        raw_names: HashSet::new(),
        failure: None,
    };
    let result = ZipStreamReader::new(reader).visit(&mut visitor);
    if let Some(error) = visitor.failure {
        return Err(error);
    }
    result.map_err(malformed)?;
    if visitor.count != usize::from(end.number_of_files) || read.get() != directory_size as u64 + 4
    {
        return Err(malformed(
            "directory count or boundary disagrees with end record",
        ));
    }
    Ok(DirectoryInfo {
        start: directory_start as u64,
        count: visitor.count,
        end: end_pos,
    })
}

pub(crate) struct DirectoryInfo {
    start: u64,
    count: usize,
    end: usize,
}

/// Ensure the ZIP readers used by EPUB see the same bounded entries and local
/// headers. This runs only after `validate_directory` and common ZIP preflight.
pub(crate) fn validate_epub_readers(data: &[u8]) -> Result<(), EngineError> {
    use futures::executor::block_on;

    let directory = validate_directory(data)?;
    if data[directory.end + 4..]
        .windows(4)
        .any(|bytes| bytes == b"PK\x05\x06")
    {
        return Err(EngineError::Unsupported(
            "EPUB ZIP comment contains an ambiguous end signature".into(),
        ));
    }
    let mut modern = zip_guard::ZipArchive::new(Cursor::new(data)).map_err(malformed)?;
    if modern.central_directory_start() != directory.start || modern.len() != directory.count {
        return Err(malformed("archive reader selected a different directory"));
    }
    if modern.has_duplicate_names() {
        return Err(EngineError::UnsafeArchive(
            "EPUB ZIP has duplicate members".into(),
        ));
    }
    modern.validate_header_consistency().map_err(malformed)?;
    let mut original = zip::ZipArchive::new(Cursor::new(data)).map_err(malformed)?;
    let asynchronous = block_on(async_zip::base::read::seek::ZipFileReader::new(
        futures::io::Cursor::new(data),
    ))
    .map_err(malformed)?;
    let entries = asynchronous.file().entries();
    if original.len() != modern.len() || entries.len() != modern.len() {
        return Err(malformed("archive readers disagree on member count"));
    }
    for (index, asynchronous) in entries.iter().enumerate() {
        let first = original.by_index_raw(index).map_err(malformed)?;
        let second = modern.by_index_data(index).map_err(malformed)?;
        let name = second.name().map_err(malformed)?;
        let async_name = asynchronous.filename().as_str().map_err(malformed)?;
        if first.name_raw() != second.name_raw()
            || first.name() != name
            || name != async_name
            || first.size() != second.size()
            || first.compressed_size() != second.compressed_size()
            || first.crc32() != second.crc32()
            || first.header_start() != second.header_start()
            || asynchronous.uncompressed_size() != second.size()
            || asynchronous.compressed_size() != second.compressed_size()
            || asynchronous.crc32() != second.crc32()
            || asynchronous.header_offset() != second.header_start()
        {
            return Err(malformed("archive readers disagree on a member"));
        }
    }
    Ok(())
}
