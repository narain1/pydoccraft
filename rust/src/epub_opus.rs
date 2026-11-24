//! Index a bounded Ogg Opus track and decode its presentation samples.
use std::{collections::HashSet, io::Cursor, ops::Range};

use hound::{SampleFormat, WavSpec};
use ogg::PacketReader;
use oxideav_opus::{
    framing_self_delim::parse_self_delimited, output_samples_per_channel, MultistreamDecoder,
    OpusHead, OpusPacket, OUTPUT_SAMPLE_RATE_HZ,
};

use super::{invalid, limited, unsupported, EngineError, MAX_CLIP_SAMPLES, MAX_FRAMES_PER_CLIP};

const MAX_PAGES: usize = 150_000;
const MAX_PACKETS: usize = 250_000;
const MAX_PACKET_BYTES: usize = 65_536;
const MAX_COMMENT_BYTES: usize = 1024 * 1024;

// The decoded family-1 samples are ordered as in RFC 7845 §5.1.1.2.
// Values are the WAVE_FORMAT_EXTENSIBLE bits for each speaker.
const FRONT_LEFT: u32 = 0x001;
const FRONT_RIGHT: u32 = 0x002;
const FRONT_CENTER: u32 = 0x004;
const LFE: u32 = 0x008;
const BACK_LEFT: u32 = 0x010;
const BACK_RIGHT: u32 = 0x020;
const BACK_CENTER: u32 = 0x100;
const SIDE_LEFT: u32 = 0x200;
const SIDE_RIGHT: u32 = 0x400;
const VORBIS_SPEAKERS: [&[u32]; 8] = [
    &[FRONT_CENTER],
    &[FRONT_LEFT, FRONT_RIGHT],
    &[FRONT_LEFT, FRONT_CENTER, FRONT_RIGHT],
    &[FRONT_LEFT, FRONT_RIGHT, BACK_LEFT, BACK_RIGHT],
    &[FRONT_LEFT, FRONT_CENTER, FRONT_RIGHT, BACK_LEFT, BACK_RIGHT],
    &[
        FRONT_LEFT,
        FRONT_CENTER,
        FRONT_RIGHT,
        BACK_LEFT,
        BACK_RIGHT,
        LFE,
    ],
    &[
        FRONT_LEFT,
        FRONT_CENTER,
        FRONT_RIGHT,
        SIDE_LEFT,
        SIDE_RIGHT,
        BACK_CENTER,
        LFE,
    ],
    &[
        FRONT_LEFT,
        FRONT_CENTER,
        FRONT_RIGHT,
        SIDE_LEFT,
        SIDE_RIGHT,
        BACK_LEFT,
        BACK_RIGHT,
        LFE,
    ],
];

pub(super) struct Track {
    links: Vec<Link>,
    speakers: u32,
    frames: usize,
    playback: Option<Playback>,
}

struct Link {
    bytes: Vec<u8>,
    packets: Vec<Packet>,
    head: OpusHead,
    timeline: Range<usize>,
    wave_order: Vec<Option<usize>>,
}

struct Playback {
    link: usize,
    decoder: MultistreamDecoder,
    next_packet: usize,
    pcm: Vec<i16>,
}

struct Packet {
    bytes: Range<usize>,
    frames: Range<usize>,
}

impl Track {
    pub(super) fn open(file: &[u8]) -> Result<Self, EngineError> {
        let mut links = Vec::new();
        let mut speakers = 0;
        let mut frames = 0;
        let mut coded_packets_left = MAX_PACKETS;
        for encoded in preflight(file)? {
            let link = Link::index(encoded, frames, &mut coded_packets_left)?;
            if link.head.channel_count > 1 {
                for flag in link.speakers() {
                    speakers |= flag;
                }
            }
            frames = link.timeline.end;
            links.push(link);
        }
        if speakers == 0 {
            speakers = FRONT_CENTER;
        }
        for link in &mut links {
            let decoded = link.speakers();
            link.wave_order = (0..u32::BITS)
                .map(|bit| 1 << bit)
                .filter(|flag| speakers & flag != 0)
                .map(|flag| {
                    if decoded.len() == 1
                        && speakers & FRONT_CENTER == 0
                        && (flag == FRONT_LEFT || flag == FRONT_RIGHT)
                    {
                        return Some(0);
                    }
                    decoded.iter().position(|&speaker| speaker == flag)
                })
                .collect();
        }
        Ok(Self {
            links,
            speakers,
            frames,
            playback: None,
        })
    }

    pub(super) fn spec(&self) -> WavSpec {
        WavSpec {
            channels: self.speakers.count_ones() as u16,
            sample_rate: OUTPUT_SAMPLE_RATE_HZ,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        }
    }

    pub(super) fn speaker_mask(&self) -> Option<u32> {
        (self.speakers.count_ones() > 2).then_some(self.speakers)
    }

    pub(super) fn frames(&self) -> usize {
        self.frames
    }

    pub(super) fn samples(&mut self, start: usize, stop: usize) -> Result<Vec<i16>, EngineError> {
        if start >= stop || stop > self.frames {
            return Err(invalid("Ogg Opus clip lies outside its presentation"));
        }
        let channels = self.speakers.count_ones() as usize;
        if stop - start > MAX_CLIP_SAMPLES / channels {
            return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
        }
        let first = self
            .links
            .partition_point(|link| link.timeline.end <= start);
        let last = self
            .links
            .partition_point(|link| link.timeline.start < stop);
        let mut playback = self.playback.take();
        let mut samples = Vec::with_capacity((stop - start) * channels);
        let mut work = MAX_FRAMES_PER_CLIP;
        for index in first..last {
            let link = &self.links[index];
            let mut state = playback
                .take()
                .filter(|state| state.link == index)
                .unwrap_or_else(|| Playback {
                    link: index,
                    decoder: MultistreamDecoder::from_head(&link.head),
                    next_packet: 0,
                    pcm: Vec::new(),
                });
            let begin = start.max(link.timeline.start) - link.timeline.start;
            let end = stop.min(link.timeline.end) - link.timeline.start;
            state.append(link, begin..end, &mut work, &mut samples)?;
            playback = Some(state);
        }
        self.playback = playback;
        Ok(samples)
    }
}

impl Link {
    fn speakers(&self) -> &'static [u32] {
        VORBIS_SPEAKERS[usize::from(self.head.channel_count - 1)]
    }

    fn index(
        file: &[u8],
        start: usize,
        coded_packets_left: &mut usize,
    ) -> Result<Self, EngineError> {
        let mut reader = PacketReader::new(Cursor::new(file));
        let id = reader.read_packet_expected().map_err(invalid)?;
        if !id.first_in_stream() || !id.last_in_page() || id.absgp_page() != 0 {
            return Err(invalid("invalid Ogg Opus identification page"));
        }
        let head = OpusHead::parse(&id.data).map_err(invalid)?;
        if head.channel_count > 8 || head.mapping_family > 1 {
            return Err(unsupported(
                "Ogg Opus playback requires channel mapping family 0 or 1",
            ));
        }
        let comments = reader.read_packet_expected().map_err(invalid)?;
        if !comments.last_in_page() || comments.absgp_page() != 0 {
            return Err(invalid("invalid Ogg Opus comment page"));
        }
        validate_comments(&comments.data)?;

        let mut bytes = Vec::new();
        let mut packets = Vec::new();
        let mut frames = 0usize;
        let mut origin = None;
        let mut prior_granule = 0;
        let end = loop {
            let packet = reader.read_packet_expected().map_err(invalid)?;
            let streams = head.mapping.stream_count;
            *coded_packets_left = coded_packets_left
                .checked_sub(usize::from(streams))
                .ok_or_else(|| limited("Ogg Opus exceeds its coded packet count limit"))?;
            let frame_count = packet_duration(&packet.data, streams)?;
            packets.push(Packet {
                bytes: bytes.len()..bytes.len() + packet.data.len(),
                frames: frames..frames + frame_count,
            });
            frames += frame_count;
            bytes.extend_from_slice(&packet.data);
            if !packet.last_in_page() {
                continue;
            }
            let granule = packet.absgp_page();
            if granule > i64::MAX as u64 {
                return Err(invalid("invalid Ogg Opus granule position"));
            }
            let base = *origin.get_or_insert_with(|| granule.saturating_sub(frames as u64));
            let next = base + frames as u64;
            if packet.last_in_stream() {
                if granule > next || granule < prior_granule {
                    return Err(invalid("invalid Ogg Opus final granule"));
                }
                break (granule - base) as usize;
            }
            if granule != next {
                return Err(invalid("Ogg Opus granules disagree with packet durations"));
            }
            prior_granule = granule;
        };
        let duration = end
            .checked_sub(usize::from(head.pre_skip))
            .filter(|frames| *frames != 0)
            .ok_or_else(|| invalid("Ogg Opus has no playable samples"))?;
        let end = start
            .checked_add(duration)
            .ok_or_else(|| limited("Ogg Opus presentation exceeds its frame limit"))?;
        Ok(Self {
            bytes,
            packets,
            head,
            timeline: start..end,
            wave_order: Vec::new(),
        })
    }
}

fn packet_duration(mut bytes: &[u8], streams: u8) -> Result<usize, EngineError> {
    let samples = |packet: &OpusPacket<'_>| {
        output_samples_per_channel(packet.toc.frame_size_tenths_ms) * packet.frame_count()
    };
    let mut expected = None;
    for _ in 1..streams {
        let parsed = parse_self_delimited(bytes).map_err(invalid)?;
        bytes = &bytes[parsed.consumed..];
        let count = samples(&parsed.packet);
        if *expected.get_or_insert(count) != count {
            return Err(invalid(
                "Ogg Opus substreams have different packet durations",
            ));
        }
    }
    let count = samples(&OpusPacket::parse(bytes).map_err(invalid)?);
    if !(120..=5760).contains(&count) {
        return Err(invalid("invalid Ogg Opus packet duration"));
    }
    if expected.is_some_and(|expected| expected != count) {
        return Err(invalid(
            "Ogg Opus substreams have different packet durations",
        ));
    }
    Ok(count)
}

impl Playback {
    fn append(
        &mut self,
        link: &Link,
        clip: Range<usize>,
        work: &mut usize,
        samples: &mut Vec<i16>,
    ) -> Result<(), EngineError> {
        let channels = usize::from(link.head.channel_count);
        let skip = usize::from(link.head.pre_skip);
        let (start, stop) = (clip.start + skip, clip.end + skip);
        let first = link
            .packets
            .partition_point(|packet| packet.frames.end <= start);
        let last = link
            .packets
            .partition_point(|packet| packet.frames.start < stop);
        if first < self.next_packet.saturating_sub(1) {
            self.decoder.reset();
            self.next_packet = 0;
        }
        *work = work
            .checked_sub(
                last.saturating_sub(self.next_packet) * usize::from(link.head.mapping.stream_count),
            )
            .ok_or_else(|| limited("Ogg Opus clip exceeds its packet decoding budget"))?;
        for index in first..last {
            while self.next_packet <= index {
                let packet = &link.packets[self.next_packet];
                let audio = self
                    .decoder
                    .decode_packet(&link.bytes[packet.bytes.clone()])
                    .map_err(invalid)?;
                if audio.pcm.len() != packet.frames.len() * channels {
                    return Err(invalid("Ogg Opus packet returned unexpected PCM geometry"));
                }
                self.pcm = audio.pcm;
                link.head.apply_gain(&mut self.pcm);
                self.next_packet += 1;
            }
            let packet = &link.packets[index];
            let left = start.saturating_sub(packet.frames.start) * channels;
            let right = (stop.min(packet.frames.end) - packet.frames.start) * channels;
            let source = &self.pcm[left..right];
            samples.extend(source.chunks_exact(channels).flat_map(|frame| {
                link.wave_order
                    .iter()
                    .map(|index| index.map_or(0, |index| frame[index]))
            }));
        }
        Ok(())
    }
}

fn validate_comments(data: &[u8]) -> Result<(), EngineError> {
    let malformed = || invalid("invalid Ogg Opus comment packet");
    let field = |bytes: &mut &[u8]| -> Result<usize, EngineError> {
        let (size, rest) = bytes.split_first_chunk::<4>().ok_or_else(malformed)?;
        *bytes = rest;
        Ok(u32::from_le_bytes(*size) as usize)
    };
    let mut comment = data.strip_prefix(b"OpusTags").ok_or_else(malformed)?;
    let vendor = field(&mut comment)?;
    comment = comment.get(vendor..).ok_or_else(malformed)?;
    let count = field(&mut comment)?;
    if count > comment.len() / 4 {
        return Err(malformed());
    }
    for _ in 0..count {
        let length = field(&mut comment)?;
        comment = comment.get(length..).ok_or_else(malformed)?;
    }
    Ok(())
}

/// Check transport bounds before the demuxer allocates complete packets.
/// Pages in a logical stream are contiguous. PacketReader checks page CRCs.
fn preflight(mut bytes: &[u8]) -> Result<Vec<&[u8]>, EngineError> {
    if bytes.is_empty() {
        return Err(invalid("truncated Ogg header"));
    }
    let mut serials = HashSet::new();
    let mut links = Vec::new();
    let mut pages_left = MAX_PAGES;
    let mut packets_left = MAX_PACKETS;
    while !bytes.is_empty() {
        let (serial, rest) = preflight_link(bytes, &mut pages_left, &mut packets_left)?;
        if !serials.insert(serial) {
            return Err(invalid("Ogg chain repeats a stream serial number"));
        }
        links.push(&bytes[..bytes.len() - rest.len()]);
        bytes = rest;
    }
    Ok(links)
}

fn preflight_link<'a>(
    mut bytes: &'a [u8],
    pages_left: &mut usize,
    packets_left: &mut usize,
) -> Result<([u8; 4], &'a [u8]), EngineError> {
    let serial: [u8; 4] = bytes
        .get(14..18)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| invalid("truncated Ogg header"))?;
    let mut partial = 0;
    let mut packets = 0;
    for index in 0..MAX_PAGES as u32 {
        *pages_left = pages_left
            .checked_sub(1)
            .ok_or_else(|| limited("Ogg Opus exceeds its page count limit"))?;
        let (header, rest) = bytes
            .split_first_chunk::<27>()
            .ok_or_else(|| invalid("truncated Ogg page header"))?;
        if &header[..5] != b"OggS\0" || header[5] & !7 != 0 {
            return Err(invalid("invalid Ogg page header"));
        }
        if header[14..18] != serial || (header[5] & 2 != 0 && index != 0) {
            return Err(unsupported("multiplexed Ogg audio"));
        }
        if header[18..22] != u32::to_le_bytes(index)
            || (header[5] & 2 != 0) != (index == 0)
            || (header[5] & 1 != 0) != (partial != 0)
        {
            return Err(invalid("invalid Ogg page sequence or continuation"));
        }
        let (segments, body) = rest
            .split_at_checked(usize::from(header[26]))
            .ok_or_else(|| invalid("truncated Ogg page lacing"))?;
        let mut size = 0;
        let before = packets;
        for &segment in segments {
            size += usize::from(segment);
            partial += usize::from(segment);
            let maximum = if packets == 1 {
                MAX_COMMENT_BYTES
            } else {
                MAX_PACKET_BYTES
            };
            if partial > maximum {
                return Err(limited("Ogg Opus packet size exceeds its limit"));
            }
            if segment < 255 {
                partial = 0;
                if packets >= 2 {
                    *packets_left = packets_left
                        .checked_sub(1)
                        .ok_or_else(|| limited("Ogg Opus exceeds its packet count limit"))?;
                }
                packets += 1;
            }
        }
        bytes = body
            .get(size..)
            .ok_or_else(|| invalid("truncated Ogg page body"))?;
        if index == 0 && !body[..size].starts_with(b"OpusHead") {
            return Err(unsupported("Ogg audio requires an Opus stream"));
        }
        if (index == 0 && (packets != 1 || partial != 0))
            || (before == 1 && (packets > 2 || (packets == 2 && partial != 0)))
        {
            return Err(invalid("invalid Ogg Opus header lacing"));
        }
        if header[5] & 4 != 0 {
            if packets < 3 || packets == before || partial != 0 {
                return Err(invalid("invalid Ogg final page"));
            }
            return Ok((serial, bytes));
        }
    }
    Err(limited("Ogg Opus exceeds its page count limit"))
}

#[cfg(test)]
mod tests {
    use super::packet_duration;

    #[test]
    fn validates_duration_of_each_coded_stream_in_an_ogg_packet() {
        // TOC config 31 uses one 20 ms CELT frame. The first stream has
        // an extra length byte before its payload, unlike the final stream.
        assert_eq!(
            packet_duration(&[0xf8, 1, 0xff, 0xf8, 0xff], 2).unwrap(),
            960
        );
        // Config 30 uses 10 ms, which conflicts with the self-delimited stream.
        assert!(packet_duration(&[0xf8, 1, 0xff, 0xf0, 0xff], 2).is_err());
        assert!(packet_duration(&[0xf8, 6, 0xff, 0xf8, 0xff], 2).is_err());
    }
}
