//! Index a bounded Ogg Opus track and decode its presentation samples.
use std::{io::Cursor, ops::Range};

use hound::{SampleFormat, WavSpec};
use ogg::PacketReader;
use oxideav_opus::{
    output_samples_per_channel, MultistreamDecoder, OpusHead, OpusPacket, OUTPUT_SAMPLE_RATE_HZ,
};

use super::{invalid, limited, unsupported, EngineError, MAX_CLIP_SAMPLES, MAX_FRAMES_PER_CLIP};

const MAX_PAGES: usize = 150_000;
const MAX_PACKETS: usize = 250_000;
const MAX_PACKET_BYTES: usize = 65_536;
const MAX_COMMENT_BYTES: usize = 1024 * 1024;

pub(super) struct Track {
    bytes: Vec<u8>,
    packets: Vec<Packet>,
    head: OpusHead,
    end: usize,
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
        preflight(file)?;
        let mut reader = PacketReader::new(Cursor::new(file));
        let id = reader.read_packet_expected().map_err(invalid)?;
        if !id.first_in_stream() || !id.last_in_page() || id.absgp_page() != 0 {
            return Err(invalid("invalid Ogg Opus identification page"));
        }
        let head = OpusHead::parse(&id.data).map_err(invalid)?;
        if head.channel_count > 2 || head.mapping.stream_count != 1 || head.mapping_family > 1 {
            return Err(unsupported("Ogg Opus playback requires mono or stereo"));
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
            if packets.len() >= MAX_PACKETS {
                return Err(limited("Ogg Opus exceeds its packet count limit"));
            }
            let encoded = OpusPacket::parse(&packet.data).map_err(invalid)?;
            let frame_count = output_samples_per_channel(encoded.toc.frame_size_tenths_ms)
                * encoded.frame_count();
            if !(120..=5760).contains(&frame_count) {
                return Err(invalid("invalid Ogg Opus packet duration"));
            }
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
        if end <= usize::from(head.pre_skip) {
            return Err(invalid("Ogg Opus has no playable samples"));
        }
        let decoder = MultistreamDecoder::from_head(&head);
        Ok(Self {
            bytes,
            packets,
            head,
            end,
            decoder,
            next_packet: 0,
            pcm: Vec::new(),
        })
    }

    pub(super) fn spec(&self) -> WavSpec {
        WavSpec {
            channels: u16::from(self.head.channel_count),
            sample_rate: OUTPUT_SAMPLE_RATE_HZ,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        }
    }

    pub(super) fn frames(&self) -> usize {
        self.end - usize::from(self.head.pre_skip)
    }

    pub(super) fn samples(&mut self, start: usize, stop: usize) -> Result<Vec<i16>, EngineError> {
        let channels = usize::from(self.head.channel_count);
        if stop - start > MAX_CLIP_SAMPLES / channels {
            return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
        }
        let skip = usize::from(self.head.pre_skip);
        let (start, stop) = (start + skip, stop + skip);
        let first = self
            .packets
            .partition_point(|packet| packet.frames.end <= start);
        let last = self
            .packets
            .partition_point(|packet| packet.frames.start < stop);
        if first < self.next_packet.saturating_sub(1) {
            self.decoder.reset();
            self.next_packet = 0;
        }
        if last.saturating_sub(self.next_packet) > MAX_FRAMES_PER_CLIP {
            return Err(limited("Ogg Opus clip exceeds its packet decoding budget"));
        }
        let mut samples = Vec::with_capacity((stop - start) * channels);
        for index in first..last {
            while self.next_packet <= index {
                let packet = &self.packets[self.next_packet];
                let audio = self
                    .decoder
                    .decode_packet(&self.bytes[packet.bytes.clone()])
                    .map_err(invalid)?;
                if audio.pcm.len() != packet.frames.len() * channels {
                    return Err(invalid("Ogg Opus packet returned unexpected PCM geometry"));
                }
                self.pcm = audio.pcm;
                self.head.apply_gain(&mut self.pcm);
                self.next_packet += 1;
            }
            let packet = &self.packets[index];
            let left = start.saturating_sub(packet.frames.start) * channels;
            let right = (stop.min(packet.frames.end) - packet.frames.start) * channels;
            samples.extend_from_slice(&self.pcm[left..right]);
        }
        Ok(samples)
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

/// Check transport bounds before the demuxer allocates complete packets. It
/// verifies page checksums and reassembles lacing across pages during indexing.
fn preflight(mut bytes: &[u8]) -> Result<(), EngineError> {
    let serial = bytes
        .get(14..18)
        .ok_or_else(|| invalid("truncated Ogg header"))?;
    let mut partial = 0;
    let mut packets = 0;
    for index in 0..MAX_PAGES as u32 {
        let (header, rest) = bytes
            .split_first_chunk::<27>()
            .ok_or_else(|| invalid("truncated Ogg page header"))?;
        if &header[..5] != b"OggS\0" || header[5] & !7 != 0 {
            return Err(invalid("invalid Ogg page header"));
        }
        if &header[14..18] != serial || (header[5] & 2 != 0 && index != 0) {
            return Err(unsupported("multiplexed or chained Ogg audio"));
        }
        if header[18..22] != index.to_le_bytes()
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
            if partial > maximum || packets >= MAX_PACKETS + 2 {
                return Err(limited("Ogg Opus packet count or size exceeds limits"));
            }
            if segment < 255 {
                partial = 0;
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
            if !bytes.is_empty() {
                return Err(unsupported("trailing data after the Ogg stream"));
            }
            if packets < 3 || packets == before || partial != 0 {
                return Err(invalid("invalid Ogg final page"));
            }
            return Ok(());
        }
    }
    Err(limited("Ogg Opus exceeds its page count limit"))
}
