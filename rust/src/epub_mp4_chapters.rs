//! Resolve local QuickTime and Nero chapter titles on the MP4 movie timeline.
use std::{borrow::Cow, io::Cursor, ops::Range, sync::Arc};

use oxideav_core::{CodecRegistry, Demuxer, StreamInfo};
use oxideav_mp4::demux::{open_typed, EditListEntry, MovieHeader, Mp4Demuxer, SampleInfo};

use super::Mp4Bounds;
use crate::epub::{
    audio::MAX_ENCODED_BYTES, invalid, limited, media::MAX_TIME, unsupported,
    validate_archive_path, Book, EngineError,
};

const MAX_CHAPTERS: usize = 4096;
const MAX_SAMPLE_BYTES: usize = 64 * 1024;
const MAX_TITLE_BYTES: usize = 1024;
const NERO_TIMESCALE: u32 = 10_000_000;

pub(crate) fn chapter_interval(
    epub: &[u8],
    path: &str,
    id: &str,
) -> Result<(u64, Option<u64>), EngineError> {
    validate_archive_path(path)?;
    let bytes = Book::open(epub)?.read(path, MAX_ENCODED_BYTES)?;
    let interval = std::panic::catch_unwind(|| resolve(bytes, id))
        .map_err(|_| invalid("malformed MP4 chapter index"))??;
    Ok((interval.start, Some(interval.end)))
}

fn resolve(bytes: Vec<u8>, id: &str) -> Result<Range<u64>, EngineError> {
    let bounds = Mp4Bounds::inspect_chapters(&bytes)?;
    let bytes: Arc<[u8]> = bytes.into();
    let demux =
        open_typed(Box::new(Cursor::new(bytes.clone())), &CodecRegistry::new()).map_err(invalid)?;
    let track = chapter_stream(&demux)?;
    let table = bounds
        .chapter_list
        .as_ref()
        .map(|range| &bytes[range.clone()]);
    if track.is_none() && table.is_none() {
        return Err(unsupported(
            "MP4 fragment IDs require a local QuickTime chapter track or chapter table",
        ));
    }
    let track_interval = track
        .map(|track| track_interval(&bytes, &bounds, &demux, track, id))
        .transpose()?
        .flatten();
    let table_interval = table
        .map(|table| table_interval(table, demux.movie_header(), id))
        .transpose()?
        .flatten();
    match (track_interval, table_interval) {
        (Some(track), Some(table)) if track != table => Err(invalid(format!(
            "MP4 chapter ID {id:?} is ambiguous across chapter indexes"
        ))),
        (Some(interval), _) | (_, Some(interval)) => Ok(interval),
        _ => Err(invalid(format!("MP4 chapter ID {id:?} is absent"))),
    }
}

fn track_interval(
    bytes: &[u8],
    bounds: &Mp4Bounds,
    demux: &Mp4Demuxer,
    chapter: &StreamInfo,
    id: &str,
) -> Result<Option<Range<u64>>, EngineError> {
    let stream = chapter.index;
    let header = demux
        .track_header(stream)
        .ok_or_else(|| invalid("missing MP4 chapter track header"))?;
    let entry = demux.sample_entry(stream, 1);
    if header.handler_type != *b"text"
        || demux.sample_entry_count(stream) != 1
        || entry.is_none_or(|entry| {
            !matches!(&entry.format, b"text" | b"tx3g") || entry.data_reference_index != 1
        })
        || chapter
            .params
            .options
            .get("dref_self_contained")
            .is_none_or(|value| value != "true")
    {
        return Err(unsupported(
            "MP4 chapters require a self-contained text track",
        ));
    }
    let samples = demux.samples(stream);
    if samples.is_empty() {
        return Err(invalid("empty MP4 chapter track"));
    }
    if samples.len() > MAX_CHAPTERS {
        return Err(limited("MP4 chapter track exceeds 4096 markers"));
    }
    let clock = ChapterClock::new(
        demux.movie_header(),
        header.media_timescale,
        demux.edit_list(stream),
    )?;
    let mut selected = None;
    let mut previous_end = 0;
    for sample in samples {
        if sample.composition_time != sample.decode_time
            || sample.composition_time != previous_end
            || sample.duration <= 0
        {
            return Err(invalid(
                "MP4 chapter samples require ordered contiguous times",
            ));
        }
        previous_end = sample
            .composition_time
            .checked_add(sample.duration)
            .ok_or_else(|| invalid("MP4 chapter sample time overflow"))?;
        if title(bytes, bounds, sample)? == id
            && selected
                .replace(sample.composition_time..previous_end)
                .is_some()
        {
            return Err(invalid(format!("MP4 chapter ID {id:?} is ambiguous")));
        }
    }
    selected
        .map(|interval| clock.interval(interval))
        .transpose()
}

fn chapter_stream(demux: &Mp4Demuxer) -> Result<Option<&StreamInfo>, EngineError> {
    let mut target = None;
    for stream in demux.streams() {
        if demux
            .track_header(stream.index)
            .is_none_or(|header| header.flags & 1 == 0)
        {
            continue;
        }
        if let Some(references) = stream.params.options.get("tref_chap") {
            for id in references.split_ascii_whitespace() {
                let id = id.parse::<u32>().map_err(invalid)?;
                if target.replace(id).is_some_and(|previous| previous != id) {
                    return Err(invalid("MP4 chapter track references are ambiguous"));
                }
            }
        }
    }
    let Some(target) = target else {
        return Ok(None);
    };
    let mut candidates = demux.streams().iter().filter(|stream| {
        demux
            .track_header(stream.index)
            .is_some_and(|header| header.track_id == target)
    });
    let stream = candidates
        .next()
        .ok_or_else(|| invalid("MP4 chapter reference points to a missing track"))?;
    if candidates.next().is_some() {
        return Err(invalid(
            "MP4 chapter reference points to a duplicate track ID",
        ));
    }
    Ok(Some(stream))
}

fn table_interval(
    table: &[u8],
    movie: MovieHeader,
    id: &str,
) -> Result<Option<Range<u64>>, EngineError> {
    let malformed = || invalid("malformed MP4 chapter table");
    let ([version, flags @ ..], table) = table.split_first_chunk::<4>().ok_or_else(malformed)?;
    if *flags != [0; 3] {
        return Err(malformed());
    }
    let (count, mut entries) = match version {
        0 => {
            let (count, entries) = table.split_first().ok_or_else(malformed)?;
            (usize::from(*count), entries)
        }
        1 => {
            let ([reserved, count @ ..], entries) =
                table.split_first_chunk::<5>().ok_or_else(malformed)?;
            if *reserved != 0 {
                return Err(malformed());
            }
            (u32::from_be_bytes(*count) as usize, entries)
        }
        _ => return Err(unsupported("unsupported MP4 chapter table version")),
    };
    if count > MAX_CHAPTERS {
        return Err(limited("MP4 chapter table exceeds 4096 markers"));
    }
    if count == 0 {
        return if entries.is_empty() {
            Ok(None)
        } else {
            Err(malformed())
        };
    }
    let clock = ChapterClock::new(movie, NERO_TIMESCALE, &[])?;
    let mut previous = None;
    let mut selected = None;
    let mut end = i64::MAX;
    for _ in 0..count {
        let (header, payload) = entries.split_first_chunk::<9>().ok_or_else(malformed)?;
        let (time, length) = header.split_first_chunk::<8>().ok_or_else(malformed)?;
        let ticks = u64::from_be_bytes(*time);
        if u128::from(ticks).div_ceil(10) > MAX_TIME {
            return Err(limited("MP4 chapter time exceeds its clock budget"));
        }
        let start = ticks as i64;
        if previous.is_some_and(|previous| start <= previous) {
            return Err(invalid(
                "MP4 chapter table starts must be strictly increasing",
            ));
        }
        if u128::from(ticks) * clock.units_per_media_tick >= clock.movie_end {
            return Err(invalid("MP4 chapter table start lies outside the movie"));
        }
        let (title, rest) = payload
            .split_at_checked(usize::from(length[0]))
            .ok_or_else(malformed)?;
        let title = std::str::from_utf8(title).map_err(|_| malformed())?;
        if title.chars().any(char::is_control) {
            return Err(malformed());
        }
        if title == id && selected.replace(start).is_some() {
            return Err(invalid(format!(
                "MP4 chapter ID {id:?} is ambiguous in the chapter table"
            )));
        }
        if previous.is_some() && selected == previous {
            end = start;
        }
        previous = Some(start);
        entries = rest;
    }
    if !entries.is_empty() {
        return Err(malformed());
    }
    selected.map(|start| clock.interval(start..end)).transpose()
}

fn title<'a>(
    bytes: &'a [u8],
    bounds: &Mp4Bounds,
    sample: SampleInfo,
) -> Result<Cow<'a, str>, EngineError> {
    let malformed = || invalid("invalid MP4 chapter title sample");
    let start = usize::try_from(sample.offset).map_err(|_| malformed())?;
    let size = sample.size as usize;
    let stop = start.checked_add(size).ok_or_else(malformed)?;
    if size > MAX_SAMPLE_BYTES {
        return Err(limited("MP4 chapter sample exceeds 64 KiB"));
    }
    if sample.sample_description_index != 1 || !bounds.contains_media(start..stop) {
        return Err(malformed());
    }
    let payload = bytes.get(start..stop).ok_or_else(malformed)?;
    let (size, payload) = payload.split_first_chunk::<2>().ok_or_else(malformed)?;
    let size = usize::from(u16::from_be_bytes(*size));
    if size > MAX_TITLE_BYTES * 2 + 2 {
        return Err(limited("MP4 chapter title exceeds its byte budget"));
    }
    let title = payload.get(..size).ok_or_else(malformed)?;
    let name = match title.split_first_chunk::<2>() {
        Some(([0xfe, 0xff] | [0xff, 0xfe], tail)) => {
            let big_endian = title[0] == 0xfe;
            let (units, remainder) = tail.as_chunks::<2>();
            if !remainder.is_empty() {
                return Err(malformed());
            }
            let units: Vec<_> = units
                .iter()
                .map(|&word| {
                    if big_endian {
                        u16::from_be_bytes(word)
                    } else {
                        u16::from_le_bytes(word)
                    }
                })
                .collect();
            Cow::Owned(String::from_utf16(&units).map_err(|_| malformed())?)
        }
        _ => Cow::Borrowed(std::str::from_utf8(title).map_err(|_| malformed())?),
    };
    if name.len() > MAX_TITLE_BYTES {
        return Err(limited("MP4 chapter title exceeds its byte budget"));
    }
    if name.chars().any(char::is_control) {
        return Err(malformed());
    }
    Ok(name)
}

// Retain both clocks until the mapped interval has been intersected with
// the movie and edit. Converting each term separately loses fractional ticks.
struct ChapterClock {
    denominator: u128,
    units_per_media_tick: u128,
    media_origin: i64,
    movie_start: u128,
    movie_end: u128,
}

impl ChapterClock {
    fn new(
        movie: MovieHeader,
        media_timescale: u32,
        edits: &[EditListEntry],
    ) -> Result<Self, EngineError> {
        if movie.timescale == 0
            || media_timescale == 0
            || matches!(movie.duration, 0 | 0xffff_ffff | u64::MAX)
        {
            return Err(invalid("MP4 chapters require a finite movie clock"));
        }
        let continuous =
            |edit: &EditListEntry| edit.media_rate_integer == 1 && edit.media_rate_fraction == 0;
        let (movie_start, media_origin, duration) = match edits {
            [] => (0, 0, movie.duration),
            [edit] if edit.media_time >= 0 && continuous(edit) => {
                (0, edit.media_time, edit.segment_duration)
            }
            [gap, edit]
                if gap.is_empty_edit()
                    && continuous(gap)
                    && edit.media_time >= 0
                    && continuous(edit) =>
            {
                (gap.segment_duration, edit.media_time, edit.segment_duration)
            }
            _ => {
                return Err(unsupported(
                    "MP4 chapter track has a noncontinuous edit list",
                ))
            }
        };
        let movie_end = movie_start.saturating_add(duration).min(movie.duration);
        if movie_start >= movie_end {
            return Err(invalid("MP4 chapter track edit has no playable interval"));
        }
        Ok(Self {
            denominator: u128::from(movie.timescale) * u128::from(media_timescale),
            units_per_media_tick: u128::from(movie.timescale),
            media_origin,
            movie_start: u128::from(movie_start) * u128::from(media_timescale),
            movie_end: u128::from(movie_end) * u128::from(media_timescale),
        })
    }

    fn interval(&self, source: Range<i64>) -> Result<Range<u64>, EngineError> {
        let bound = |time: i64| {
            let ticks = time.saturating_sub(self.media_origin).max(0) as u128;
            let movie = (self.movie_start + ticks * self.units_per_media_tick).min(self.movie_end);
            let micros = (movie * 1_000_000).div_ceil(self.denominator);
            if micros > MAX_TIME {
                return Err(limited("MP4 chapter time exceeds its clock budget"));
            }
            Ok(micros as u64)
        };
        let interval = bound(source.start)?..bound(source.end)?;
        if interval.is_empty() {
            return Err(unsupported("MP4 chapter has no playable interval"));
        }
        Ok(interval)
    }
}
