//! Validate Ogg transport and select a logical stream from each physical segment.
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use ogg::{
    reading::{BasePacketReader, OggPage, PageParser},
    Packet,
};

use super::{invalid, limited, unsupported, EngineError};

pub(super) const MAX_PACKETS: usize = 250_000;
const MAX_PAGES: usize = 150_000;
const MAX_PACKET_BYTES: usize = 65_536;
const MAX_COMMENT_BYTES: usize = 1024 * 1024;
const MAX_VIDEO_PACKET_BYTES: usize = 2 * 1024 * 1024;
const MAX_SKELETON_BYTES: usize = 1024 * 1024;
const MAX_SELECTED_LINKS: usize = 256;
const MAX_NAMED_STREAMS: usize = 256;
const CONTINUED: u8 = 1;
const BOS: u8 = 2;
const EOS: u8 = 4;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Codec {
    Opus,
    Vorbis,
    Theora,
}

impl Codec {
    fn from_identification(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"OpusHead") {
            Some(Self::Opus)
        } else if bytes.starts_with(b"\x01vorbis") {
            Some(Self::Vorbis)
        } else if bytes.starts_with(b"\x80theora") {
            Some(Self::Theora)
        } else {
            None
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Opus => "Opus",
            Self::Vorbis => "Vorbis",
            Self::Theora => "Theora",
        }
    }
}

type Serial = [u8; 4];

#[derive(Default)]
pub(super) struct SelectedTracks {
    audio: Vec<(Serial, Codec)>,
    video: Vec<Serial>,
}

impl SelectedTracks {
    pub(super) fn audio_codec(&self) -> Option<Codec> {
        let &(_, codec) = self.audio.first()?;
        self.audio
            .iter()
            .all(|&(_, value)| value == codec)
            .then_some(codec)
    }

    pub(super) fn mixed_audio(&self) -> bool {
        !self.audio.is_empty() && self.audio_codec().is_none()
    }

    pub(super) fn has_video(&self) -> bool {
        !self.video.is_empty()
    }

    fn track(&self, index: usize, codec: Codec) -> Result<Serial, EngineError> {
        let serial = match codec {
            Codec::Theora => self.video.get(index).copied(),
            codec => self
                .audio
                .get(index)
                .filter(|&&(_, selected)| selected == codec)
                .map(|&(serial, _)| serial),
        };
        serial.ok_or_else(|| {
            unsupported("named Ogg fragment does not select the requested media type")
        })
    }
}

pub(super) struct Segment<'a> {
    pub pages: Vec<Page<'a>>,
    pub codec: Codec,
    pub opus_tracks: usize,
    pub vorbis_tracks: usize,
    pub has_other_audio: bool,
}

pub(super) struct Page<'a> {
    header: &'a [u8; 27],
    lacing: &'a [u8],
    body: &'a [u8],
}

impl<'a> Page<'a> {
    fn read(bytes: &'a [u8]) -> Result<(Self, &'a [u8]), EngineError> {
        let (header, rest) = bytes
            .split_first_chunk::<27>()
            .ok_or_else(|| invalid("truncated Ogg page header"))?;
        if &header[..5] != b"OggS\0" || header[5] & !(CONTINUED | BOS | EOS) != 0 {
            return Err(invalid("invalid Ogg page header"));
        }
        let (lacing, rest) = rest
            .split_at_checked(usize::from(header[26]))
            .ok_or_else(|| invalid("truncated Ogg page lacing"))?;
        let length = lacing.iter().map(|&value| usize::from(value)).sum();
        let (body, rest) = rest
            .split_at_checked(length)
            .ok_or_else(|| invalid("truncated Ogg page body"))?;
        Ok((
            Self {
                header,
                lacing,
                body,
            },
            rest,
        ))
    }

    pub(super) fn parse(self) -> Result<OggPage, EngineError> {
        let (mut parser, _) = PageParser::new(*self.header).map_err(invalid)?;
        parser.parse_segments(self.lacing.to_vec());
        parser
            .parse_packet_data(self.body.to_vec())
            .map_err(invalid)
    }

    pub(super) fn empty_eos_granule(&self) -> Option<u64> {
        (self.flag(EOS) && self.lacing.is_empty())
            .then(|| u64::from_le_bytes(self.header[6..14].try_into().expect("Ogg granule field")))
    }

    fn serial(&self) -> Serial {
        self.header[14..18]
            .try_into()
            .expect("Ogg page serial field")
    }

    fn identifies_other_audio(&self) -> bool {
        self.body.starts_with(b"\x7fFLAC") || self.body.starts_with(b"Speex   ")
    }

    fn flag(&self, flag: u8) -> bool {
        self.header[5] & flag != 0
    }
}

struct Stream {
    codec: Option<Codec>,
    selected: bool,
    next_page: u32,
    partial: usize,
    packets: usize,
}

impl Stream {
    fn accept(&mut self, page: &Page<'_>, packets_left: &mut usize) -> Result<(), EngineError> {
        if page.header[18..22] != self.next_page.to_le_bytes()
            || page.flag(CONTINUED) != (self.partial != 0)
        {
            return Err(invalid("invalid Ogg page sequence or continuation"));
        }
        let before = self.packets;
        let opus = self.codec == Some(Codec::Opus);
        for &length in page.lacing {
            self.partial += usize::from(length);
            let maximum = if self.packets == 1 {
                MAX_COMMENT_BYTES
            } else {
                MAX_PACKET_BYTES
            };
            if opus && self.partial > maximum {
                return Err(limited("Ogg Opus packet size exceeds its limit"));
            }
            if self.codec == Some(Codec::Vorbis) && self.partial > maximum {
                return Err(limited("Ogg Vorbis packet size exceeds its limit"));
            }
            if self.selected
                && self.codec == Some(Codec::Theora)
                && self.partial > MAX_VIDEO_PACKET_BYTES
            {
                return Err(limited("Ogg Theora packet exceeds 2 MiB"));
            }
            if length < 255 {
                self.partial = 0;
                if !opus || self.packets >= 2 {
                    *packets_left = packets_left
                        .checked_sub(1)
                        .ok_or_else(|| limited("Ogg exceeds its packet count limit"))?;
                }
                self.packets += 1;
            }
        }
        if page.flag(BOS) && (self.packets != 1 || self.partial != 0) {
            return Err(invalid("invalid Ogg identification page lacing"));
        }
        if opus && before == 1 && (self.packets > 2 || (self.packets == 2 && self.partial != 0)) {
            return Err(invalid("invalid Ogg Opus comment page lacing"));
        }
        if page.flag(EOS)
            && (self.partial != 0 || (opus && (self.packets < 3 || self.packets == before)))
        {
            return Err(invalid("invalid Ogg final page"));
        }
        self.next_page += 1;
        Ok(())
    }
}

/// Each segment starts with adjacent BOS pages and ends after every stream's EOS.
/// Retain the selected track's page slices. Check the other pages without assembling packets.
pub(super) fn groups<'a>(
    bytes: &'a [u8],
    codec: Codec,
    track: Option<usize>,
    named: Option<&SelectedTracks>,
) -> Result<Vec<Segment<'a>>, EngineError> {
    scan_groups(bytes, track, |index| {
        let serial = named.map(|tracks| tracks.track(index, codec)).transpose()?;
        Ok((codec, serial))
    })
}

pub(super) fn selected_audio_groups<'a>(
    bytes: &'a [u8],
    selection: &SelectedTracks,
) -> Result<Vec<Segment<'a>>, EngineError> {
    scan_groups(bytes, None, |index| {
        let &(serial, codec) = selection
            .audio
            .get(index)
            .ok_or_else(|| unsupported("named Ogg fragment does not select an audio track"))?;
        Ok((codec, Some(serial)))
    })
}

fn scan_groups<'a>(
    mut bytes: &'a [u8],
    track: Option<usize>,
    pick: impl Fn(usize) -> Result<(Codec, Option<Serial>), EngineError>,
) -> Result<Vec<Segment<'a>>, EngineError> {
    if bytes.is_empty() {
        return Err(invalid("truncated Ogg header"));
    }
    let mut serials = HashSet::new();
    let mut active = HashMap::new();
    let mut groups = Vec::new();
    let mut selected = Vec::new();
    let mut beginning = true;
    let mut tracks = 0;
    let mut opus_tracks = 0;
    let mut vorbis_tracks = 0;
    let mut has_other_audio = false;
    let mut packets_left = MAX_PACKETS;
    let (mut codec, mut named) = pick(0)?;
    for _ in 0..MAX_PAGES {
        let (page, rest) = Page::read(bytes)?;
        bytes = rest;
        let serial = page.serial();
        if page.flag(BOS) {
            if !beginning {
                return Err(invalid("Ogg stream begins after its segment headers"));
            }
            if !serials.insert(serial) {
                return Err(invalid("Ogg repeats a stream serial number"));
            }
            let identified = Codec::from_identification(page.body);
            let matches = identified == Some(codec);
            let selected_track = matches
                && named.map_or_else(|| track.unwrap_or(0) == tracks, |target| serial == target);
            tracks += usize::from(matches);
            opus_tracks += usize::from(identified == Some(Codec::Opus));
            vorbis_tracks += usize::from(identified == Some(Codec::Vorbis));
            has_other_audio |= page.identifies_other_audio();
            active.insert(
                serial,
                Stream {
                    codec: identified,
                    selected: selected_track,
                    next_page: 0,
                    partial: 0,
                    packets: 0,
                },
            );
        } else {
            beginning = false;
        }
        let stream = active
            .get_mut(&serial)
            .ok_or_else(|| invalid("Ogg page belongs to an inactive stream"))?;
        stream.accept(&page, &mut packets_left)?;
        let selected_track = stream.selected;
        if page.flag(EOS) {
            active.remove(&serial);
        }
        if selected_track {
            selected.push(page);
        } else {
            page.parse()?;
        }
        // A one-page auxiliary stream can finish on its BOS page while
        // the remaining identification pages of the same segment follow.
        let more_headers = beginning && bytes.get(5).is_some_and(|flags| flags & BOS != 0);
        if active.is_empty() && !more_headers {
            if tracks == 0 {
                return Err(unsupported(match codec {
                    Codec::Opus => "Ogg segment requires an Opus stream",
                    Codec::Vorbis => "Ogg segment requires a Vorbis stream",
                    Codec::Theora => "Ogg segment requires a Theora stream",
                }));
            }
            if named.is_none() && track.is_none() && tracks > 1 {
                let message = match codec {
                    Codec::Opus => {
                        "multiple Opus tracks in one Ogg segment require an explicit selection"
                    }
                    Codec::Theora => {
                        "multiple Theora tracks in one Ogg segment require an explicit selection"
                    }
                    Codec::Vorbis => "multiple Vorbis tracks in one Ogg segment",
                };
                return Err(unsupported(message));
            }
            if selected.is_empty() {
                return Err(EngineError::InvalidArgument(format!(
                    "{} track {} is absent from Ogg segment {} ({} tracks)",
                    codec.name(),
                    track.unwrap_or(0),
                    groups.len(),
                    tracks,
                )));
            }
            groups.push(Segment {
                pages: std::mem::take(&mut selected),
                codec,
                opus_tracks,
                vorbis_tracks,
                has_other_audio,
            });
            beginning = true;
            tracks = 0;
            opus_tracks = 0;
            vorbis_tracks = 0;
            has_other_audio = false;
            if !bytes.is_empty() {
                (codec, named) = pick(groups.len())?;
            }
        }
        if bytes.is_empty() {
            if !active.is_empty() {
                return Err(invalid("truncated Ogg segment"));
            }
            return Ok(groups);
        }
    }
    Err(limited("Ogg exceeds its page count limit"))
}

struct DefaultLink {
    audio: (Serial, Codec),
    video: Option<Serial>,
    skeleton: Option<Serial>,
    repeated_skeleton: bool,
    encoded: Range<usize>,
}

/// Look for a codec change between links with one genuine default soundtrack.
/// Uniform and ambiguous selections keep the existing per-codec rules.
pub(super) fn default_mixed_tracks(
    encoded: &[u8],
    video_track: Option<usize>,
) -> Result<Option<SelectedTracks>, EngineError> {
    if encoded.is_empty() {
        return Ok(None);
    }
    let mut remaining = encoded;
    let mut active = HashSet::new();
    let mut links = Vec::<DefaultLink>::new();
    let mut audio = None;
    let mut video = None;
    let mut video_tracks = 0;
    let mut skeleton = None;
    let mut repeated_skeleton = false;
    let mut beginning = true;
    let mut start = 0;
    let mut mixed = false;
    let mut too_many = false;
    for _ in 0..MAX_PAGES {
        let (page, rest) = Page::read(remaining)?;
        remaining = rest;
        let serial = page.serial();
        if page.flag(BOS) {
            if !beginning {
                return Err(invalid("Ogg stream begins after its segment headers"));
            }
            if !active.insert(serial) {
                return Err(invalid("Ogg segment repeats a stream serial"));
            }
            match Codec::from_identification(page.body) {
                Some(Codec::Opus | Codec::Vorbis) if audio.is_some() => return Ok(None),
                Some(codec @ (Codec::Opus | Codec::Vorbis)) => {
                    audio = Some((serial, codec));
                }
                Some(Codec::Theora) => {
                    if video_track.unwrap_or(0) == video_tracks {
                        video = Some(serial);
                    }
                    video_tracks += 1;
                }
                _ if page.identifies_other_audio() => return Ok(None),
                _ if page.body.starts_with(b"fishead\0") => {
                    repeated_skeleton |= skeleton.replace(serial).is_some();
                }
                _ => {}
            }
        } else {
            beginning = false;
        }
        if !active.contains(&serial) {
            return Err(invalid("Ogg page belongs to an inactive stream"));
        }
        if page.flag(EOS) {
            active.remove(&serial);
        }
        let more_headers = beginning && remaining.get(5).is_some_and(|flags| flags & BOS != 0);
        if active.is_empty() && !more_headers {
            let Some(audio) = audio.take() else {
                return Ok(None);
            };
            mixed |= links.first().is_some_and(|first| first.audio.1 != audio.1);
            let end = encoded.len() - remaining.len();
            if links.len() == MAX_SELECTED_LINKS {
                too_many = true;
            } else {
                links.push(DefaultLink {
                    audio,
                    video,
                    skeleton,
                    repeated_skeleton,
                    encoded: start..end,
                });
            }
            video = None;
            video_tracks = 0;
            skeleton = None;
            repeated_skeleton = false;
            beginning = true;
            start = end;
        }
        if remaining.is_empty() {
            if !active.is_empty() {
                return Err(invalid("truncated Ogg segment"));
            }
            return if mixed {
                if too_many {
                    return Err(limited("mixed Ogg exceeds its segment count limit"));
                }
                describe_defaults(encoded, links).map(Some)
            } else {
                Ok(None)
            };
        }
    }
    Err(limited("Ogg exceeds its page count limit"))
}

fn describe_defaults(
    encoded: &[u8],
    links: Vec<DefaultLink>,
) -> Result<SelectedTracks, EngineError> {
    let mut selection = SelectedTracks::default();
    let mut title = None;
    let mut metadata_bytes = 0;
    for link in links {
        if link.repeated_skeleton {
            return Err(invalid("Ogg segment repeats its Skeleton stream"));
        }
        if let Some(skeleton) = link.skeleton {
            let mut remaining = &encoded[link.encoded.clone()];
            let mut pages = Vec::new();
            while !remaining.is_empty() {
                let (page, rest) = Page::read(remaining)?;
                if page.serial() == skeleton {
                    super::charge(
                        &mut metadata_bytes,
                        remaining.len() - rest.len(),
                        MAX_SKELETON_BYTES,
                        "mixed Ogg Skeleton headers exceed 1 MiB",
                    )?;
                    pages.push(page);
                }
                remaining = rest;
            }
            if let Some(name) = default_skeleton_name(pages, &link)? {
                if title.get_or_insert_with(|| name.clone()) != &name {
                    return Err(unsupported(
                        "mixed Ogg soundtrack changes its Skeleton Name",
                    ));
                }
            }
        }
        selection.audio.push(link.audio);
    }
    Ok(selection)
}

fn default_skeleton_name(
    pages: Vec<Page<'_>>,
    link: &DefaultLink,
) -> Result<Option<String>, EngineError> {
    let mut described = HashSet::new();
    let mut title = None;
    for packet in skeleton_packets(pages, "mixed Ogg")? {
        let packet = packet?;
        if skeleton_index_or_eos(&packet) {
            continue;
        }
        let (serial, name) = skeleton_name(&packet.data)?;
        if !described.insert(serial) {
            return Err(invalid("Ogg Skeleton repeats a track header"));
        }
        if serial == link.audio.0 || Some(serial) == link.video {
            if packet.data[36..44] != [0; 8] {
                return Err(unsupported(
                    "mixed Ogg requires a zero Skeleton track base granule",
                ));
            }
            if serial == link.audio.0 {
                title = name.map(str::to_owned);
            }
        }
    }
    Ok(title)
}

/// Resolve names through each segment's bounded Skeleton headers. The selected
/// decoder subsequently validates transport and limits for all the streams.
pub(super) fn named_tracks(
    mut bytes: &[u8],
    names: &[String],
) -> Result<SelectedTracks, EngineError> {
    let mut selection = SelectedTracks::default();
    let mut active = HashSet::new();
    let mut codecs = HashMap::new();
    let mut skeleton = None;
    let mut metadata = Vec::new();
    let mut metadata_bytes = 0usize;
    let mut audio_name = None;
    let mut link = 0;
    let mut beginning = true;
    for _ in 0..MAX_PAGES {
        let (page, rest) = Page::read(bytes)?;
        bytes = rest;
        let serial = page.serial();
        if page.flag(BOS) {
            if !beginning {
                return Err(invalid("Ogg stream begins after its segment headers"));
            }
            if codecs.len() >= MAX_NAMED_STREAMS {
                return Err(limited("named Ogg segment exceeds its stream count limit"));
            }
            if page.body.starts_with(b"fishead\0") && skeleton.replace(serial).is_some() {
                return Err(invalid("Ogg segment repeats its Skeleton stream"));
            }
            if codecs
                .insert(serial, Codec::from_identification(page.body))
                .is_some()
            {
                return Err(invalid("Ogg segment repeats a stream serial"));
            }
            active.insert(serial);
        } else {
            beginning = false;
        }
        if !active.contains(&serial) {
            return Err(invalid("Ogg page belongs to an inactive stream"));
        }
        let eos = page.flag(EOS);
        if Some(serial) == skeleton {
            super::charge(
                &mut metadata_bytes,
                page.header.len() + page.lacing.len() + page.body.len(),
                MAX_SKELETON_BYTES,
                "named Ogg Skeleton headers exceed 1 MiB",
            )?;
            metadata.push(page);
        }
        if eos {
            active.remove(&serial);
        }
        let more_headers = beginning && bytes.get(5).is_some_and(|flags| flags & BOS != 0);
        if active.is_empty() && !more_headers {
            if link >= MAX_SELECTED_LINKS {
                return Err(limited("named Ogg exceeds its segment count limit"));
            }
            let NamedSegment { audio, video } =
                resolve_names(std::mem::take(&mut metadata), &codecs, names)?;
            let name = audio.map(|track| track.name);
            if link > 0 && (audio_name != name || selection.has_video() != video.is_some()) {
                return Err(unsupported(
                    "named Ogg tracks change media type across segments",
                ));
            }
            if let Some(track) = audio {
                selection.audio.push((track.serial, track.codec));
                audio_name = name;
            }
            if let Some(serial) = video {
                selection.video.push(serial);
            }
            link += 1;
            skeleton = None;
            beginning = true;
            codecs.clear();
        }
        if bytes.is_empty() {
            return if active.is_empty() {
                Ok(selection)
            } else {
                Err(invalid("truncated Ogg segment"))
            };
        }
    }
    Err(limited("Ogg exceeds its page count limit"))
}

#[derive(Clone, Copy)]
struct NamedAudio {
    serial: Serial,
    codec: Codec,
    name: usize,
}

struct NamedSegment {
    audio: Option<NamedAudio>,
    video: Option<Serial>,
}

fn resolve_names(
    pages: Vec<Page<'_>>,
    codecs: &HashMap<Serial, Option<Codec>>,
    names: &[String],
) -> Result<NamedSegment, EngineError> {
    if pages.is_empty() {
        return Err(unsupported(
            "named Ogg fragments require Skeleton track names",
        ));
    }
    let packets = skeleton_packets(pages, "named Ogg")?;
    let mut matched = vec![None; names.len()];
    let mut described = HashSet::new();
    for packet in packets {
        let packet = packet?;
        if skeleton_index_or_eos(&packet) {
            continue;
        }
        let (serial, name) = skeleton_name(&packet.data)?;
        let codec = *codecs
            .get(&serial)
            .ok_or_else(|| invalid("Ogg Skeleton names an absent stream"))?;
        if !described.insert(serial) {
            return Err(invalid("Ogg Skeleton repeats a track header"));
        }
        if let Some(index) = names
            .iter()
            .position(|requested| Some(requested.as_str()) == name)
        {
            if packet.data[36..44] != [0; 8] {
                return Err(unsupported(
                    "named Ogg requires a zero Skeleton track base granule",
                ));
            }
            if matched[index].replace((serial, codec)).is_some() {
                return Err(invalid(format!(
                    "Ogg track name {:?} is ambiguous",
                    names[index]
                )));
            }
        }
    }
    let mut audio = None;
    let mut video = None;
    for (name, (label, entry)) in names.iter().zip(matched).enumerate() {
        let (serial, codec) =
            entry.ok_or_else(|| invalid(format!("Ogg track name {label:?} is absent")))?;
        let unique = match codec {
            Some(Codec::Theora) => video.replace(serial).is_none(),
            Some(codec @ (Codec::Opus | Codec::Vorbis)) => audio
                .replace(NamedAudio {
                    serial,
                    codec,
                    name,
                })
                .is_none(),
            None => {
                return Err(unsupported(
                    "named Ogg fragment requires Opus, Vorbis or Theora",
                ))
            }
        };
        if !unique {
            return Err(unsupported(
                "named Ogg fragment selects multiple tracks of the same media type",
            ));
        }
    }
    Ok(NamedSegment { audio, video })
}

fn skeleton_packets<'a>(
    pages: Vec<Page<'a>>,
    selection: &str,
) -> Result<impl Iterator<Item = Result<Packet, EngineError>> + 'a, EngineError> {
    let mut packets = packets(pages);
    let head = packets
        .next()
        .transpose()?
        .ok_or_else(|| invalid("missing Ogg Skeleton header"))?;
    if !head.first_in_stream()
        || !head.data.starts_with(b"fishead\0")
        || head.data.len() < 64
        || !matches!(head.data[8..12], [3 | 4, 0, 0, 0])
    {
        return Err(unsupported(&format!(
            "{selection} requires Skeleton version 3.0 or 4.0"
        )));
    }
    if head.data[12..20] != [0; 8] || head.data[28..36] != [0; 8] {
        return Err(unsupported(&format!(
            "{selection} requires a zero Skeleton time origin"
        )));
    }
    Ok(packets)
}

fn skeleton_index_or_eos(packet: &Packet) -> bool {
    (packet.data.is_empty() && packet.last_in_stream()) || packet.data.starts_with(b"index\0")
}

fn skeleton_name(bytes: &[u8]) -> Result<(Serial, Option<&str>), EngineError> {
    if !bytes.starts_with(b"fisbone\0") || bytes.len() < 52 {
        return Err(invalid("invalid Ogg Skeleton track header"));
    }
    let offset =
        u32::from_le_bytes(bytes[8..12].try_into().expect("Skeleton header offset")) as usize;
    let headers = offset
        .checked_add(8)
        .filter(|&start| start >= 52)
        .and_then(|start| bytes.get(start..))
        .ok_or_else(|| invalid("invalid Ogg Skeleton message header offset"))?;
    let serial = bytes[12..16].try_into().expect("Skeleton track serial");
    let lines = std::str::from_utf8(headers).map_err(invalid)?;
    let mut picked = None;
    for line in lines.split("\r\n") {
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("Name") {
                let name = value.trim_matches([' ', '\t']);
                if name.is_empty()
                    || name.len() > 1024
                    || name.chars().any(char::is_control)
                    || picked.replace(name).is_some()
                {
                    return Err(invalid("invalid or repeated Ogg Skeleton track Name"));
                }
            }
        }
    }
    Ok((serial, picked))
}

/// Recognize the first segment before choosing the full, checksum-checked reader.
/// Automatic selection preserves preference for Opus when both codecs are available.
pub(super) fn audio_codec(mut bytes: &[u8]) -> Result<Codec, EngineError> {
    let mut codec = Codec::Opus;
    for _ in 0..MAX_PAGES {
        let (page, rest) = Page::read(bytes)?;
        if !page.flag(BOS) {
            return Ok(codec);
        }
        match Codec::from_identification(page.body) {
            Some(Codec::Opus) => return Ok(Codec::Opus),
            Some(Codec::Vorbis) => codec = Codec::Vorbis,
            _ => {}
        }
        if rest.is_empty() {
            return Ok(codec);
        }
        bytes = rest;
    }
    Err(limited("Ogg exceeds its page count limit"))
}

pub(super) fn packets(
    pages: Vec<Page<'_>>,
) -> impl Iterator<Item = Result<Packet, EngineError>> + '_ {
    let mut pages = pages.into_iter();
    let mut reader = BasePacketReader::new();
    std::iter::from_fn(move || loop {
        if let Some(packet) = reader.read_packet() {
            return Some(Ok(packet));
        }
        let page = pages.next()?;
        if let Err(error) = page
            .parse()
            .and_then(|page| reader.push_page(page).map_err(invalid))
        {
            return Some(Err(error));
        }
    })
}
