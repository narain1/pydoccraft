//! Bound local MP4 metadata and locate sample storage before demuxing.
use std::{io::Cursor, ops::Range};

use oxideav_mp4::boxes::read_box_header;

use super::{invalid, limited, unsupported, EngineError};

#[path = "epub_mp4_chapters.rs"]
mod chapters;

pub(crate) use chapters::chapter_interval;

const MAX_METADATA_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_ACCESS_UNITS: usize = 250_000;

#[derive(Default)]
pub(super) struct Mp4Bounds {
    boxes: usize,
    metadata: usize,
    samples: usize,
    media: Vec<Range<usize>>,
    chapter_list: Option<Range<usize>>,
    f4v_chapter_list: Option<Range<usize>>,
}

impl Mp4Bounds {
    pub(super) fn contains_media(&self, range: Range<usize>) -> bool {
        let index = self.media.partition_point(|mdat| mdat.end <= range.start);
        self.media
            .get(index)
            .is_some_and(|mdat| mdat.start <= range.start && range.end <= mdat.end)
    }

    pub(super) fn inspect(bytes: &[u8]) -> Result<Self, EngineError> {
        Self::inspect_boxes(bytes, None)
    }

    fn inspect_chapters(bytes: &[u8]) -> Result<Self, EngineError> {
        Self::inspect_boxes(bytes, Some([0; 4]))
    }

    // Track the parent box when preflighting chapter metadata.
    fn inspect_boxes(bytes: &[u8], parent: Option<[u8; 4]>) -> Result<Self, EngineError> {
        let mut bounds = Self::default();
        bounds.check(bytes, 0, 0, parent)?;
        Ok(bounds)
    }

    fn check(
        &mut self,
        bytes: &[u8],
        base: usize,
        depth: usize,
        parent: Option<[u8; 4]>,
    ) -> Result<(), EngineError> {
        if depth > 8 {
            return Err(limited("MP4 box nesting exceeds limits"));
        }
        let mut cursor = Cursor::new(bytes);
        let mut track_references = false;
        let mut chapter_references = false;
        while cursor.position() < bytes.len() as u64 {
            let header = read_box_header(&mut cursor)
                .map_err(invalid)?
                .ok_or_else(|| invalid("truncated MP4 box"))?;
            let start = cursor.position() as usize;
            let length = usize::try_from(
                header
                    .payload_size()
                    .unwrap_or((bytes.len() - start) as u64),
            )
            .map_err(invalid)?;
            let end = start
                .checked_add(length)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| invalid("MP4 box extends outside its parent"))?;
            self.boxes += 1;
            if self.boxes > 20_000 {
                return Err(limited("too many MP4 boxes"));
            }
            if depth == 0 && header.fourcc == *b"mdat" {
                self.media.push(base + start..base + end);
            } else if depth == 0 {
                self.metadata += length;
                if self.metadata > MAX_METADATA_BYTES {
                    return Err(limited("MP4 metadata exceeds 4 MiB"));
                }
            }
            if parent == Some(*b"tref")
                && &header.fourcc == b"chap"
                && (std::mem::replace(&mut chapter_references, true)
                    || length == 0
                    || !length.is_multiple_of(4))
            {
                return Err(invalid("invalid or repeated MP4 chapter reference"));
            }
            if &header.fourcc == b"chpl" {
                let table = match parent {
                    Some(parent) if &parent == b"udta" => Some(&mut self.chapter_list),
                    Some(parent) if &parent == b"moov" && depth == 1 => {
                        Some(&mut self.f4v_chapter_list)
                    }
                    _ => None,
                };
                if let Some(table) = table {
                    if header.payload_size().is_none() {
                        return Err(invalid("MP4 chapter table requires an explicit box size"));
                    }
                    if table.replace(base + start..base + end).is_some() {
                        return Err(invalid("duplicate MP4 chapter table"));
                    }
                }
            }
            let child_parent = parent.map(|_| header.fourcc);
            match &header.fourcc {
                b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" => {
                    self.check(&bytes[start..end], base + start, depth + 1, child_parent)?;
                }
                b"udta" if parent == Some(*b"moov") && depth == 1 => {
                    self.check(&bytes[start..end], base + start, depth + 1, child_parent)?;
                }
                b"tref" if parent == Some(*b"trak") => {
                    if std::mem::replace(&mut track_references, true) {
                        return Err(invalid("duplicate MP4 track reference box"));
                    }
                    self.check(&bytes[start..end], base + start, depth + 1, child_parent)?;
                }
                b"stsz" | b"stz2" => {
                    let raw = bytes[start..end]
                        .get(8..12)
                        .ok_or_else(|| invalid("truncated MP4 sample count"))?;
                    self.samples += u32::from_be_bytes(raw.try_into().map_err(invalid)?) as usize;
                    if self.samples > MAX_ACCESS_UNITS {
                        return Err(limited("too many MP4 access units"));
                    }
                }
                b"moof" | b"mvex" => {
                    return Err(unsupported("fragmented MP4 media is unsupported"))
                }
                _ => {}
            }
            cursor.set_position(end as u64);
        }
        Ok(())
    }
}
