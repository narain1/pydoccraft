//! Bounded WebM transport shared by the publication's audio and video readers.
use std::{collections::HashSet, io::Cursor};

use oxideav_core::{CodecRegistry, Demuxer, Error, MediaType, Packet, TimeBase};
use oxideav_mkv::{
    demux::{open_typed, Chapter, Edition, MkvDemuxer},
    ebml::{read_element_header, read_uint, read_vint, VINT_UNKNOWN_SIZE},
    ids,
    schema::{element_def, ElementType},
};

use super::{
    audio::MAX_ENCODED_BYTES, invalid, limited, media::integer, unsupported, validate_archive_path,
    Book, EngineError,
};

const MAX_ELEMENTS: usize = 400_000;
const MAX_PACKETS: usize = 250_000;
const MAX_BLOCK_BYTES: usize = 2 * 1024 * 1024;
const MAX_TRACKS: usize = 32;
const MAX_TICKS: u64 = 1 << 52;
pub(super) const MAX_TIME_NS: u64 = (1 << 53) - 1;

pub(super) struct Reader {
    pub demux: MkvDemuxer,
    blocks: std::vec::IntoIter<Block>,
    tracks: Vec<u64>,
    block: Option<(Block, u16)>,
}

#[derive(Clone, Copy)]
struct Block {
    track: u64,
    count: u16,
}

pub(super) struct Unit {
    pub packet: Packet,
    pub index_in_block: u16,
    pub block_count: u16,
    pub discard_padding: i64,
}

impl Reader {
    pub(super) fn open(bytes: Vec<u8>) -> Result<Self, EngineError> {
        Self::open_bounded(bytes, false)
    }

    fn open_bounded(bytes: Vec<u8>, chapter_fields: bool) -> Result<Self, EngineError> {
        let mut bounds = Bounds {
            chapter_fields,
            ..Bounds::default()
        };
        bounds.walk(&bytes, None, false, 0)?;
        if !bounds.webm || !bounds.segment {
            return Err(unsupported("embedded WebM requires a WebM EBML segment"));
        }
        if bounds.tracks.is_empty() || bounds.blocks.is_empty() {
            return Err(invalid("WebM has no media packets"));
        }
        let distinct: HashSet<_> = bounds.tracks.iter().copied().collect();
        if distinct.len() != bounds.tracks.len() {
            return Err(invalid("WebM repeats a track number"));
        }
        let mut demux =
            open_typed(Box::new(Cursor::new(bytes)), &CodecRegistry::new()).map_err(invalid)?;
        if demux.streams().len() != bounds.tracks.len() {
            return Err(invalid("WebM track index disagrees with its container"));
        }
        if chapter_fields && bounds.late_chapters && demux.chapters().is_empty() {
            return Err(unsupported("late WebM chapters require a valid SeekHead"));
        }
        demux.set_apply_track_operations(false);
        Ok(Self {
            demux,
            blocks: bounds.blocks.into_iter(),
            tracks: bounds.tracks,
            block: None,
        })
    }

    pub(super) fn track(
        &self,
        kind: MediaType,
        selection: Option<usize>,
        names: &[String],
    ) -> Result<u32, EngineError> {
        let mut streams = self
            .demux
            .streams()
            .iter()
            .filter(|stream| stream.params.media_type == kind);
        let label = if kind == MediaType::Video {
            "video"
        } else {
            "audio"
        };
        let stream = if !names.is_empty() {
            let mut video = None;
            let mut audio = None;
            for name in names {
                let mut matched = self.demux.streams().iter().filter(|stream| {
                    self.demux
                        .track_identity(stream.index)
                        .and_then(|track| track.name())
                        == Some(name.as_str())
                });
                let entry = matched
                    .next()
                    .ok_or_else(|| invalid(format!("WebM track name {name:?} is absent")))?;
                if matched.next().is_some() {
                    return Err(invalid(format!("WebM track name {name:?} is ambiguous")));
                }
                let slot = match entry.params.media_type {
                    MediaType::Audio => &mut audio,
                    MediaType::Video => &mut video,
                    _ => {
                        return Err(unsupported(
                            "named WebM fragments require audio or video tracks",
                        ))
                    }
                };
                if slot.replace(entry).is_some() {
                    return Err(unsupported(
                        "named WebM fragment selects multiple tracks of the same media type",
                    ));
                }
            }
            let picked = if kind == MediaType::Video {
                video
            } else {
                audio
            };
            picked.ok_or_else(|| {
                unsupported("named WebM fragment does not select the requested media type")
            })?
        } else if let Some(ordinal) = selection {
            streams.nth(ordinal).ok_or_else(|| {
                EngineError::InvalidArgument(format!("WebM {label} track {ordinal} is absent"))
            })?
        } else {
            let stream = streams.next().ok_or_else(|| {
                unsupported(match kind {
                    MediaType::Video => "WebM has no video track",
                    _ => "WebM has no audio track",
                })
            })?;
            if streams.next().is_some() {
                return Err(unsupported("multiple WebM tracks of the same media type"));
            }
            stream
        };
        if self
            .demux
            .track_timing(stream.index)
            .is_some_and(|t| t.track_timestamp_scale() != 1.0)
        {
            return Err(unsupported("WebM track timestamp scaling"));
        }
        Ok(stream.index)
    }

    pub(super) fn next(&mut self) -> Result<Option<Unit>, EngineError> {
        let expected = self
            .block
            .take()
            .or_else(|| self.blocks.next().map(|block| (block, 0)));
        let packet = match self.demux.next_packet() {
            Ok(packet) => packet,
            Err(Error::Eof) if expected.is_none() => return Ok(None),
            Err(error) => return Err(invalid(error)),
        };
        let (block, index) = expected.ok_or_else(|| invalid("WebM returned an unindexed frame"))?;
        if self.tracks.get(packet.stream_index as usize) != Some(&block.track) {
            return Err(invalid("WebM packet order disagrees with its blocks"));
        }
        if index + 1 < block.count {
            self.block = Some((block, index + 1));
        }
        Ok(Some(Unit {
            packet,
            index_in_block: index,
            block_count: block.count,
            discard_padding: self
                .demux
                .block_group_meta()
                .and_then(|meta| meta.discard_padding())
                .unwrap_or(0),
        }))
    }
}

pub(crate) fn chapter_interval(
    epub: &[u8],
    path: &str,
    id: &str,
) -> Result<(u64, Option<u64>), EngineError> {
    validate_archive_path(path)?;
    let bytes = Book::open(epub)?.read(path, MAX_ENCODED_BYTES)?;
    std::panic::catch_unwind(|| {
        let reader = Reader::open_bounded(bytes, true)?;
        if reader
            .demux
            .crc_status()
            .iter()
            .any(|status| status.element_id == ids::CHAPTERS && !status.is_valid())
        {
            return Err(invalid("WebM chapter checksum mismatch"));
        }
        chapter_bounds(reader.demux.chapters(), id)
    })
    .map_err(|_| invalid("malformed WebM chapter index"))?
}

type ChapterLocation<'a> = (&'a [Chapter], usize);

fn chapter_bounds(editions: &[Edition], id: &str) -> Result<(u64, Option<u64>), EngineError> {
    let numeric = integer(id);
    let mut selected = None;
    for edition in editions {
        if let Some(path) = chapter_path(&edition.chapters, id, numeric)? {
            if selected.replace((edition, path)).is_some() {
                return Err(invalid(format!("WebM chapter ID {id:?} is ambiguous")));
            }
        }
    }
    let (edition, path) =
        selected.ok_or_else(|| invalid(format!("WebM chapter ID {id:?} is absent")))?;
    if edition.ordered {
        return Err(unsupported("WebM chapter IDs in an ordered edition"));
    }
    let mut start = 0;
    let mut end: Option<u64> = None;
    for (siblings, index) in path.into_iter().rev() {
        let chapter = &siblings[index];
        if chapter.segment_uuid.is_some() || chapter.segment_edition_uid.is_some() {
            return Err(unsupported("WebM chapter IDs that link to another segment"));
        }
        if !chapter.chap_processes.is_empty() || !chapter.track_uids.is_empty() {
            return Err(unsupported("WebM chapter commands or track restrictions"));
        }
        if !chapter.enabled {
            return Err(unsupported("disabled WebM chapter"));
        }
        if chapter.uid.is_none()
            || chapter.time_start_ns < start
            || end.is_some_and(|end| chapter.time_start_ns > end)
        {
            return Err(invalid("invalid WebM chapter identity or nested interval"));
        }
        start = chapter.time_start_ns;
        let declared = chapter.time_end_ns.or_else(|| {
            siblings
                .iter()
                .map(|sibling| sibling.time_start_ns)
                .filter(|&time| time > start)
                .min()
        });
        if declared.is_some_and(|end| end < start) {
            return Err(invalid("WebM chapter end precedes its start"));
        }
        if start > MAX_TIME_NS || declared.is_some_and(|end| end > MAX_TIME_NS) {
            return Err(limited(
                "WebM chapter timestamps exceed their timing budget",
            ));
        }
        end = end.into_iter().chain(declared).min();
    }
    let start = start.div_ceil(1000);
    let end = end.map(|end| end.div_ceil(1000));
    if end == Some(start) {
        return Err(unsupported("WebM chapter marker has no playable interval"));
    }
    Ok((start, end))
}

fn chapter_path<'a>(
    chapters: &'a [Chapter],
    id: &str,
    numeric: Option<u64>,
) -> Result<Option<Vec<ChapterLocation<'a>>>, EngineError> {
    let mut matched = None;
    for (index, chapter) in chapters.iter().enumerate() {
        let mut path = chapter_path(&chapter.children, id, numeric)?;
        let matches = chapter.string_uid.as_deref() == Some(id)
            || numeric.is_some_and(|uid| chapter.uid == Some(uid));
        if matches && path.replace(Vec::new()).is_some() {
            return Err(invalid(format!("WebM chapter ID {id:?} is ambiguous")));
        }
        if let Some(mut path) = path {
            path.push((chapters, index));
            if matched.replace(path).is_some() {
                return Err(invalid(format!("WebM chapter ID {id:?} is ambiguous")));
            }
        }
    }
    Ok(matched)
}

pub(super) fn timestamp(scale: TimeBase, ticks: i64) -> Result<i128, EngineError> {
    if scale.num() <= 0 || scale.den() <= 0 {
        return Err(invalid("invalid WebM time base"));
    }
    let nanos =
        i128::from(ticks) * i128::from(scale.num()) * 1_000_000_000 / i128::from(scale.den());
    if nanos.abs() > i128::from(MAX_TIME_NS) {
        return Err(limited("WebM timestamps exceed their timing budget"));
    }
    Ok(nanos)
}

impl Unit {
    pub(super) fn time(&self) -> Result<i128, EngineError> {
        timestamp(
            self.packet.time_base,
            self.packet
                .pts
                .ok_or_else(|| invalid("WebM block has no timestamp"))?,
        )
    }
}

#[derive(Default)]
struct Bounds {
    webm: bool,
    segment: bool,
    nodes: usize,
    packets: usize,
    tracks: Vec<u64>,
    blocks: Vec<Block>,
    chapter_fields: bool,
    seen_cluster: bool,
    late_chapters: bool,
}

impl Bounds {
    fn walk<'a>(
        &mut self,
        mut bytes: &'a [u8],
        parent: Option<u32>,
        unknown: bool,
        depth: usize,
    ) -> Result<&'a [u8], EngineError> {
        if depth > 24 {
            return Err(limited("WebM element nesting exceeds 24 levels"));
        }
        let chapter_atom = self.chapter_fields && parent == Some(ids::CHAPTER_ATOM);
        let mut chapter_fields = 0;
        while !bytes.is_empty() {
            let before = bytes;
            let header = read_element_header(&mut bytes).map_err(invalid)?;
            let definition = element_def(header.id);
            if chapter_atom {
                let field = match header.id {
                    ids::CHAPTER_UID => 1,
                    ids::CHAPTER_TIME_START => 2,
                    ids::CHAPTER_TIME_END => 4,
                    ids::CHAPTER_STRING_UID => 8,
                    _ => 0,
                };
                if chapter_fields & field != 0 {
                    return Err(invalid("WebM chapter repeats an ID or timestamp field"));
                }
                chapter_fields |= field;
            }
            if unknown
                && parent == Some(ids::CLUSTER)
                && definition.is_some_and(|def| def.parent_id == Some(ids::SEGMENT))
            {
                return Ok(before);
            }
            super::charge(
                &mut self.nodes,
                1,
                MAX_ELEMENTS,
                "WebM exceeds its element count limit",
            )?;
            if matches!(
                header.id,
                ids::TRACK_OPERATION
                    | ids::CONTENT_ENCODINGS
                    | ids::ATTACHMENTS
                    | ids::ENCRYPTED_BLOCK
                    | ids::CODEC_STATE
                    | ids::BLOCK_ADDITIONS
            ) {
                return Err(unsupported(
                    "WebM track transforms, encryption, attachments or block extensions",
                ));
            }
            if definition.is_some_and(|def| {
                !matches!(header.id, ids::VOID | ids::CRC32)
                    && def.parent_id != parent
                    && !(def.recursive && parent == Some(header.id))
            }) {
                return Err(invalid("WebM element is outside its declared parent"));
            }
            if self.chapter_fields {
                match header.id {
                    ids::CLUSTER => self.seen_cluster = true,
                    ids::CHAPTERS => self.late_chapters = self.seen_cluster,
                    _ => {}
                }
            }
            if header.id == ids::SEGMENT && std::mem::replace(&mut self.segment, true) {
                return Err(unsupported("multiple WebM segments"));
            }
            if header.size == VINT_UNKNOWN_SIZE {
                if !matches!(header.id, ids::SEGMENT | ids::CLUSTER) {
                    return Err(invalid("unexpected unknown-size WebM element"));
                }
                bytes = self.walk(bytes, Some(header.id), true, depth + 1)?;
                continue;
            }
            let length = usize::try_from(header.size)
                .map_err(|_| limited("WebM element size exceeds limits"))?;
            let (body, rest) = bytes
                .split_at_checked(length)
                .ok_or_else(|| invalid("truncated WebM element"))?;
            if definition.is_some_and(|def| def.element_type == ElementType::Master) {
                self.walk(body, Some(header.id), false, depth + 1)?;
            } else {
                self.leaf(header.id, body)?;
            }
            bytes = rest;
        }
        if chapter_atom && chapter_fields & 3 != 3 {
            return Err(invalid(
                "WebM chapter lacks a ChapterUID or start timestamp",
            ));
        }
        Ok(bytes)
    }

    fn leaf(&mut self, id: u32, mut bytes: &[u8]) -> Result<(), EngineError> {
        let size = bytes.len();
        match id {
            ids::EBML_DOC_TYPE => self.webm = bytes == b"webm",
            ids::TRACK_NUMBER => {
                let track = read_uint(&mut bytes, size).map_err(invalid)?;
                if track == 0 || self.tracks.len() == MAX_TRACKS {
                    return Err(limited("WebM exceeds its track number limits"));
                }
                self.tracks.push(track);
            }
            ids::TIMECODE | ids::BLOCK_DURATION | ids::TIMECODE_SCALE => {
                let value = read_uint(&mut bytes, size).map_err(invalid)?;
                if value > MAX_TICKS
                    || id == ids::TIMECODE_SCALE && !(1..=1_000_000_000).contains(&value)
                {
                    return Err(limited("WebM time code exceeds its limits"));
                }
            }
            ids::SIMPLE_BLOCK | ids::BLOCK => {
                if bytes.len() > MAX_BLOCK_BYTES {
                    return Err(limited("WebM coded block exceeds 2 MiB"));
                }
                let (track, _) = read_vint(&mut bytes, false).map_err(invalid)?;
                let flags = *bytes
                    .get(2)
                    .ok_or_else(|| invalid("short WebM block header"))?;
                let count = if flags & 0x06 == 0 {
                    1
                } else {
                    1 + u16::from(
                        *bytes
                            .get(3)
                            .ok_or_else(|| invalid("short WebM lacing header"))?,
                    )
                };
                super::charge(
                    &mut self.packets,
                    usize::from(count),
                    MAX_PACKETS,
                    "WebM exceeds its packet count limit",
                )?;
                self.blocks.push(Block { track, count });
            }
            _ => {}
        }
        Ok(())
    }
}
