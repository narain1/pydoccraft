//! Validate Ogg transport and select a logical stream from each physical segment.
use std::collections::{HashMap, HashSet};

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

pub(super) struct Segment<'a> {
    pub pages: Vec<Page<'a>>,
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
pub(super) fn groups(
    mut bytes: &[u8],
    codec: Codec,
    track: Option<usize>,
) -> Result<Vec<Segment<'_>>, EngineError> {
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
    for _ in 0..MAX_PAGES {
        let (page, rest) = Page::read(bytes)?;
        bytes = rest;
        let serial = [
            page.header[14],
            page.header[15],
            page.header[16],
            page.header[17],
        ];
        if page.flag(BOS) {
            if !beginning {
                return Err(invalid("Ogg stream begins after its segment headers"));
            }
            if !serials.insert(serial) {
                return Err(invalid("Ogg repeats a stream serial number"));
            }
            let identified = Codec::from_identification(page.body);
            let matches = identified == Some(codec);
            let selected_track = matches && track.unwrap_or(0) == tracks;
            tracks += usize::from(matches);
            opus_tracks += usize::from(identified == Some(Codec::Opus));
            vorbis_tracks += usize::from(identified == Some(Codec::Vorbis));
            has_other_audio |=
                page.body.starts_with(b"\x7fFLAC") || page.body.starts_with(b"Speex   ");
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
            if track.is_none() && tracks > 1 {
                let message = match codec {
                    Codec::Opus => {
                        "multiple Opus tracks in one Ogg segment require an explicit selection"
                    }
                    Codec::Theora => "multiple Theora tracks in one Ogg segment",
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
                opus_tracks,
                vorbis_tracks,
                has_other_audio,
            });
            beginning = true;
            tracks = 0;
            opus_tracks = 0;
            vorbis_tracks = 0;
            has_other_audio = false;
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
