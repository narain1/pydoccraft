//! Validate Ogg transport and select a single Opus stream from each physical segment.
use std::collections::{HashMap, HashSet};

use ogg::reading::{OggPage, PageParser};

use super::{invalid, limited, unsupported, EngineError, MAX_PACKETS};

const MAX_PAGES: usize = 150_000;
const MAX_PACKET_BYTES: usize = 65_536;
const MAX_COMMENT_BYTES: usize = 1024 * 1024;
const CONTINUED: u8 = 1;
const BOS: u8 = 2;
const EOS: u8 = 4;

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

    fn flag(&self, flag: u8) -> bool {
        self.header[5] & flag != 0
    }
}

struct Stream {
    opus: bool,
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
        for &length in page.lacing {
            self.partial += usize::from(length);
            let maximum = if self.packets == 1 {
                MAX_COMMENT_BYTES
            } else {
                MAX_PACKET_BYTES
            };
            if self.opus && self.partial > maximum {
                return Err(limited("Ogg Opus packet size exceeds its limit"));
            }
            if length < 255 {
                self.partial = 0;
                if !self.opus || self.packets >= 2 {
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
        if self.opus
            && before == 1
            && (self.packets > 2 || (self.packets == 2 && self.partial != 0))
        {
            return Err(invalid("invalid Ogg Opus comment page lacing"));
        }
        if page.flag(EOS)
            && (self.partial != 0 || (self.opus && (self.packets < 3 || self.packets == before)))
        {
            return Err(invalid("invalid Ogg final page"));
        }
        self.next_page += 1;
        Ok(())
    }
}

/// Each segment starts with adjacent BOS pages and ends after every stream's EOS.
/// Retain only Opus page slices. Auxiliary checksums are verified without assembling packets.
pub(super) fn groups(mut bytes: &[u8]) -> Result<Vec<Vec<Page<'_>>>, EngineError> {
    if bytes.is_empty() {
        return Err(invalid("truncated Ogg header"));
    }
    let mut serials = HashSet::new();
    let mut active = HashMap::new();
    let mut groups = Vec::new();
    let mut selected = Vec::new();
    let mut beginning = true;
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
            let opus = page.body.starts_with(b"OpusHead");
            if opus && !selected.is_empty() {
                return Err(unsupported("multiple Opus tracks in one Ogg segment"));
            }
            active.insert(
                serial,
                Stream {
                    opus,
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
        let opus = stream.opus;
        if page.flag(EOS) {
            active.remove(&serial);
        }
        if opus {
            selected.push(page);
        } else {
            page.parse()?;
        }
        // A one-page auxiliary stream can finish on its BOS page while
        // the remaining identification pages of the same segment follow.
        let more_headers = beginning && bytes.get(5).is_some_and(|flags| flags & BOS != 0);
        if active.is_empty() && !more_headers {
            if selected.is_empty() {
                return Err(unsupported("Ogg segment requires an Opus stream"));
            }
            groups.push(std::mem::take(&mut selected));
            beginning = true;
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
