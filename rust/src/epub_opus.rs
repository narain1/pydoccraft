//! Index bounded Opus tracks and decode their presentation samples.
use std::ops::Range;

use hound::{SampleFormat, WavSpec};
use oxideav_core::Demuxer;
use oxideav_opus::{
    framing_self_delim::parse_self_delimited, output_samples_per_channel, MultistreamDecoder,
    OpusHead, OpusPacket, OUTPUT_SAMPLE_RATE_HZ,
};

use super::{
    invalid, limited, unsupported, EngineError, FRONT_CENTER, FRONT_LEFT, FRONT_RIGHT,
    MAX_CLIP_SAMPLES, MAX_FRAMES_PER_CLIP, VORBIS_SPEAKERS,
};

use super::super::{
    ogg::{self as ogg_pages, Codec, NamedTracks, MAX_PACKETS},
    webm,
};

pub(in crate::epub) struct Track {
    links: Vec<Link>,
    speakers: u32,
    frames: usize,
    playback: Option<Playback>,
}

struct Link {
    bytes: Vec<u8>,
    packets: Vec<Packet>,
    head: OpusHead,
    pre_skip: usize,
    timeline: Range<usize>,
    playable: usize,
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
    pub(in crate::epub) fn open(
        file: &[u8],
        opus_track: Option<usize>,
        named: Option<&NamedTracks>,
    ) -> Result<Self, EngineError> {
        let mut links = Vec::new();
        let mut frames = 0;
        let mut coded_packets_left = MAX_PACKETS;
        for encoded in ogg_pages::groups(file, Codec::Opus, opus_track, named)? {
            let link = Link::index(encoded.pages, frames, &mut coded_packets_left)?;
            frames = link.timeline.end;
            links.push(link);
        }
        Ok(Self::from_links(links, frames))
    }

    pub(super) fn open_ogg_link(
        pages: Vec<ogg_pages::Page<'_>>,
        packets_left: &mut usize,
    ) -> Result<Self, EngineError> {
        let link = Link::index(pages, 0, packets_left)?;
        let frames = link.playable;
        Ok(Self::from_links(vec![link], frames))
    }

    pub(super) fn release_playback(&mut self) {
        self.playback = None;
    }

    pub(super) fn open_webm(mut reader: webm::Reader, index: u32) -> Result<Self, EngineError> {
        let stream = &reader.demux.streams()[index as usize];
        if stream.params.extradata.len() > 1024 {
            return Err(limited("WebM Opus identification header exceeds 1 KiB"));
        }
        let head = OpusHead::parse(&stream.params.extradata).map_err(invalid)?;
        if head.channel_count > 8 || head.mapping_family > 1 {
            return Err(unsupported(
                "WebM Opus playback requires channel mapping family 0 or 1",
            ));
        }
        if stream.params.channels != Some(u16::from(head.channel_count)) {
            return Err(invalid("WebM Opus channel count disagrees with its header"));
        }
        let delay = reader
            .demux
            .track_codec_timing(index)
            .map_or(0, |t| t.codec_delay());
        let time_of =
            |frames: usize| (frames as i128 * 1_000_000_000) / i128::from(OUTPUT_SAMPLE_RATE_HZ);
        if (i128::from(delay) - time_of(usize::from(head.pre_skip))).abs() > 1 {
            return Err(invalid("WebM Opus codec delay disagrees with its pre-skip"));
        }
        let mut bytes = Vec::new();
        let mut packets = Vec::new();
        let mut frames = 0usize;
        let mut origin = None;
        let mut end_padding = 0;
        let mut block_start = 0;
        let mut budget = 0;
        while let Some(unit) = reader.next()? {
            if unit.packet.stream_index != index {
                continue;
            }
            if end_padding != 0 || unit.discard_padding < 0 {
                return Err(unsupported(
                    "WebM Opus discard padding outside the final block",
                ));
            }
            if unit.index_in_block == 0 {
                block_start = frames;
                let time = unit.time()?;
                let start = *origin.get_or_insert(time);
                let tick = webm::timestamp(unit.packet.time_base, 1)?;
                if (time - start - time_of(frames)).abs() > tick + 1 {
                    return Err(unsupported("discontinuous WebM Opus timestamps"));
                }
            }
            let streams = head.mapping.stream_count;
            super::super::charge(
                &mut budget,
                usize::from(streams),
                MAX_PACKETS,
                "WebM Opus exceeds its coded packet count limit",
            )?;
            let count = packet_duration(&unit.packet.data, streams)?;
            packets.push(Packet {
                bytes: bytes.len()..bytes.len() + unit.packet.data.len(),
                frames: frames..frames + count,
            });
            frames += count;
            bytes.extend_from_slice(&unit.packet.data);
            if unit.index_in_block + 1 == unit.block_count {
                end_padding = webm_audio_frames(unit.discard_padding as u64)?;
                if end_padding > frames - block_start {
                    return Err(invalid("WebM Opus discard padding exceeds its final block"));
                }
            }
        }
        let origin = origin.ok_or_else(|| invalid("WebM Opus has no coded packets"))?;
        let start = webm_audio_frames(origin.max(0) as u64)?;
        let pre_skip = usize::from(head.pre_skip)
            .checked_add(webm_audio_frames((-origin).max(0) as u64)?)
            .ok_or_else(|| limited("WebM Opus pre-skip exceeds limits"))?;
        let end_trim = end_padding
            .checked_add(pre_skip)
            .ok_or_else(|| limited("WebM Opus trim exceeds limits"))?;
        let playable = frames
            .checked_sub(end_trim)
            .filter(|&n| n != 0)
            .ok_or_else(|| invalid("WebM Opus has no playable samples"))?;
        let end = start
            .checked_add(playable)
            .ok_or_else(|| limited("WebM Opus presentation exceeds limits"))?;
        let link = Link {
            bytes,
            packets,
            head,
            pre_skip,
            timeline: start..end,
            playable,
            wave_order: Vec::new(),
        };
        Ok(Self::from_links(vec![link], end))
    }

    fn from_links(mut links: Vec<Link>, frames: usize) -> Self {
        let mut speakers = links
            .iter()
            .filter(|link| link.head.channel_count > 1)
            .flat_map(Link::speakers)
            .fold(0, |mask, &flag| mask | flag);
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
        Self {
            links,
            speakers,
            frames,
            playback: None,
        }
    }

    pub(in crate::epub) fn link_lengths(&self) -> impl ExactSizeIterator<Item = usize> + '_ {
        self.links.iter().map(|link| link.playable)
    }

    pub(in crate::epub) fn align_video_links(
        &mut self,
        video_frames: &[usize],
    ) -> Result<(), EngineError> {
        if video_frames.len() != self.links.len() {
            return Err(invalid("Ogg video and audio segments disagree"));
        }
        let mut cursor = 0usize;
        let last = self.links.len().saturating_sub(1);
        for (index, (link, pictures)) in self.links.iter_mut().zip(video_frames).enumerate() {
            let duration = if index < last {
                link.playable.max(*pictures)
            } else {
                link.playable
            };
            let end = cursor
                .checked_add(duration)
                .ok_or_else(|| limited("Ogg chain duration exceeds limits"))?;
            link.timeline = cursor..end;
            cursor = end;
        }
        self.frames = cursor;
        self.playback = None;
        Ok(())
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
        self.samples_with_work(start, stop, &mut 0)
    }

    pub(super) fn samples_with_work(
        &mut self,
        start: usize,
        stop: usize,
        work: &mut usize,
    ) -> Result<Vec<i16>, EngineError> {
        if start >= stop || stop > self.frames {
            return Err(invalid("Opus clip lies outside its presentation"));
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
        let leading = self.links[0].timeline.start.min(stop).saturating_sub(start);
        samples.resize(leading * channels, 0);
        for index in first..last {
            let link = &self.links[index];
            let begin = start.max(link.timeline.start) - link.timeline.start;
            let end = stop.min(link.timeline.end) - link.timeline.start;
            let audible = end.min(link.playable);
            if begin < audible {
                let mut state = playback
                    .take()
                    .filter(|state| state.link == index)
                    .unwrap_or_else(|| Playback {
                        link: index,
                        decoder: MultistreamDecoder::from_head(&link.head),
                        next_packet: 0,
                        pcm: Vec::new(),
                    });
                state.append(link, begin..audible, work, &mut samples)?;
                playback = Some(state);
            }
            samples.resize(samples.len() + (end - audible.max(begin)) * channels, 0);
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
        pages: Vec<ogg_pages::Page<'_>>,
        start: usize,
        coded_packets_left: &mut usize,
    ) -> Result<Self, EngineError> {
        let mut packets = ogg_pages::packets(pages);
        let mut next_packet = || {
            packets
                .next()
                .transpose()?
                .ok_or_else(|| invalid("truncated Ogg Opus stream"))
        };
        let id = next_packet()?;
        if !id.first_in_stream() || !id.last_in_page() || id.absgp_page() != 0 {
            return Err(invalid("invalid Ogg Opus identification page"));
        }
        let head = OpusHead::parse(&id.data).map_err(invalid)?;
        if head.channel_count > 8 || head.mapping_family > 1 {
            return Err(unsupported(
                "Ogg Opus playback requires channel mapping family 0 or 1",
            ));
        }
        let comments = next_packet()?;
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
            let packet = next_packet()?;
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
        let pre_skip = usize::from(head.pre_skip);
        let duration = end
            .checked_sub(pre_skip)
            .filter(|frames| *frames != 0)
            .ok_or_else(|| invalid("Ogg Opus has no playable samples"))?;
        let end = start
            .checked_add(duration)
            .ok_or_else(|| limited("Ogg Opus presentation exceeds its frame limit"))?;
        Ok(Self {
            bytes,
            packets,
            head,
            pre_skip,
            timeline: start..end,
            playable: duration,
            wave_order: Vec::new(),
        })
    }
}

fn webm_audio_frames(nanos: u64) -> Result<usize, EngineError> {
    if nanos > webm::MAX_TIME_NS {
        return Err(limited("WebM audio time exceeds its timing budget"));
    }
    usize::try_from(
        (u128::from(nanos) * u128::from(OUTPUT_SAMPLE_RATE_HZ) + 500_000_000) / 1_000_000_000,
    )
    .map_err(|_| limited("WebM audio time exceeds its frame budget"))
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
        let skip = link.pre_skip;
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
        // Scale the Opus budget to the work units shared with Vorbis.
        super::super::charge(
            work,
            last.saturating_sub(self.next_packet) * usize::from(link.head.mapping.stream_count) * 2,
            MAX_FRAMES_PER_CLIP * 2,
            "Ogg Opus clip exceeds its packet decoding budget",
        )?;
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
