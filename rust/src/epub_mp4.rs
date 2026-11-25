//! Bound local MP4 metadata and locate sample storage before demuxing.
use std::{io::Cursor, ops::Range};

use oxideav_mp4::{
    boxes::read_box_header,
    demux::{EditListEntry, MovieHeader},
};

use super::{invalid, limited, media::MAX_TIME, unsupported, EngineError};

#[path = "epub_mp4_chapters.rs"]
mod chapters;

pub(crate) use chapters::chapter_interval;

const MAX_METADATA_BYTES: usize = 4 * 1024 * 1024;
const MAX_EDIT_LIST_ENTRIES: u32 = 64;
pub(super) const MAX_ACCESS_UNITS: usize = 250_000;

#[derive(Clone, Copy)]
pub(super) struct LinearEdit {
    pub movie_start: u64,
    pub movie_end: u64,
    pub media_start: i64,
}

impl LinearEdit {
    pub(super) fn implicit_tail_end(&self, movie: MovieHeader) -> Result<Option<u64>, EngineError> {
        // ISO BMFF places a track's final empty edit outside its edit list.
        // The supported rate-1 edit list already gives the track duration.
        if matches!(movie.duration, 0 | 0xffff_ffff | u64::MAX) || movie.duration <= self.movie_end
        {
            return Ok(None);
        }
        if movie.timescale == 0 {
            return Err(invalid("MP4 timescale is zero"));
        }
        if u128::from(movie.duration) * 1_000_000 > MAX_TIME * u128::from(movie.timescale) {
            return Err(limited("MP4 presentation exceeds its clock budget"));
        }
        Ok(Some(movie.duration))
    }
}

/// Collapse adjacent source edits and allow one forward jump between rate-1 intervals.
pub(super) fn edit_spans(
    edits: &[EditListEntry],
    movie_timescale: u32,
    media_timescale: u32,
    unsupported_layout: &str,
) -> Result<Vec<LinearEdit>, EngineError> {
    let rate_one =
        |edit: &EditListEntry| edit.media_rate_integer == 1 && edit.media_rate_fraction == 0;
    let Some((head, rest)) = edits.split_first() else {
        return Ok(Vec::new());
    };
    let (movie_start, playable) = if head.is_empty_edit() && rate_one(head) {
        (head.segment_duration, rest)
    } else {
        (0, edits)
    };
    let (first, rest) = playable
        .split_first()
        .filter(|(first, _)| first.media_time >= 0 && rate_one(first))
        .ok_or_else(|| unsupported(unsupported_layout))?;
    if !rest.is_empty() && (movie_timescale == 0 || media_timescale == 0) {
        return Err(invalid("invalid MP4 edit-list timescale"));
    }

    let mut spans = Vec::new();
    let mut span = LinearEdit {
        movie_start,
        movie_end: movie_start,
        media_start: first.media_time,
    };
    let mut previous = first;
    for next in rest {
        span.movie_end = span
            .movie_end
            .checked_add(previous.segment_duration)
            .ok_or_else(|| invalid("MP4 edit duration overflow"))?;
        if next.media_time < 0
            || !rate_one(next)
            || previous.segment_duration == 0
            || next.segment_duration == 0
        {
            return Err(unsupported(unsupported_layout));
        }
        let elapsed_media = i128::from(next.media_time) - i128::from(span.media_start);
        let elapsed_movie = i128::from(span.movie_end - span.movie_start);
        match (elapsed_media * i128::from(movie_timescale))
            .cmp(&(elapsed_movie * i128::from(media_timescale)))
        {
            std::cmp::Ordering::Less => return Err(unsupported(unsupported_layout)),
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => {
                if !spans.is_empty() {
                    return Err(unsupported(
                        "MP4 edit lists support one forward source jump",
                    ));
                }
                spans.push(span);
                span = LinearEdit {
                    movie_start: span.movie_end,
                    movie_end: span.movie_end,
                    media_start: next.media_time,
                };
            }
        }
        previous = next;
    }
    span.movie_end = span
        .movie_end
        .checked_add(previous.segment_duration)
        .ok_or_else(|| invalid("MP4 edit duration overflow"))?;
    spans.push(span);
    Ok(spans)
}

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
                b"elst" => {
                    let count = bytes[start..end]
                        .get(4..8)
                        .ok_or_else(|| invalid("truncated MP4 edit count"))?;
                    if u32::from_be_bytes(count.try_into().map_err(invalid)?)
                        > MAX_EDIT_LIST_ENTRIES
                    {
                        return Err(limited("MP4 edit list exceeds 64 entries"));
                    }
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
