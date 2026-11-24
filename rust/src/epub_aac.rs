//! Bounded AAC-LC decoding on the MP4 presentation timeline.
use std::{io::Cursor, ops::Range, sync::Arc};

use hound::{SampleFormat, WavSpec};
use oxideav_aac::{asc::AudioSpecificConfig, decode::StreamDecoder};
use oxideav_core::{CodecRegistry, Demuxer, MediaType};
use oxideav_mp4::{boxes::read_box_header, demux::open_typed};

use super::{frame_at, invalid, limited, unsupported, EngineError, MAX_CLIP_SAMPLES};

const MAX_METADATA_BYTES: usize = 4 * 1024 * 1024;
const MAX_ACCESS_UNITS: usize = 250_000;
const MAX_PACKET_BYTES: usize = 32_768;

pub(super) struct Track {
    bytes: Arc<[u8]>,
    packets: Vec<Range<usize>>,
    config: AudioSpecificConfig,
    spec: WavSpec,
    media: Range<usize>,
    decoder: StreamDecoder,
    next_packet: usize,
    pcm: Vec<i16>,
}

impl Track {
    pub(super) fn open(bytes: Vec<u8>) -> Result<Self, EngineError> {
        let mut guard = Mp4Bounds::default();
        guard.check(&bytes, 0, 0)?;
        let bytes: Arc<[u8]> = bytes.into();
        let demux = open_typed(Box::new(Cursor::new(bytes.clone())), &CodecRegistry::new())
            .map_err(invalid)?;
        let mut tracks = demux
            .streams()
            .iter()
            .filter(|stream| stream.params.media_type == MediaType::Audio);
        let stream = tracks
            .next()
            .ok_or_else(|| unsupported("MP4 narration requires an audio track"))?;
        if tracks.next().is_some() {
            return Err(unsupported("MP4 narration requires one audio track"));
        }
        let entry = demux.sample_entry(stream.index, 1);
        if demux.sample_entry_count(stream.index) != 1
            || entry.is_none_or(|entry| entry.format != *b"mp4a" || entry.data_reference_index != 1)
            || stream.params.extradata.len() > 256
        {
            return Err(unsupported("MP4 narration requires local AAC-LC audio"));
        }
        let (config, _) = AudioSpecificConfig::parse(&stream.params.extradata).map_err(invalid)?;
        if config.outer_aot != 2
            || config.sbr_present
            || config.sampling_frequency_index >= 13
            || config.ga_body.depends_on_core_coder
            || !(1..=7).contains(&config.channel_configuration)
            || !(7_350..=96_000).contains(&config.sample_rate)
        {
            return Err(unsupported(
                "MP4 narration requires AAC-LC with standard channel mapping",
            ));
        }
        let spec = WavSpec {
            channels: config.channel_count() as u16,
            sample_rate: config.sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let per_packet = config.ga_body.frame_length.samples() as usize;
        let samples = demux.samples(stream.index);
        if samples.is_empty() || samples.len() > MAX_ACCESS_UNITS {
            return Err(limited("MP4 narration access unit count is out of range"));
        }
        let time_base = stream.time_base.0;
        let media_frames = |timestamp: i64| -> Result<usize, EngineError> {
            if timestamp < 0 || time_base.num <= 0 || time_base.den <= 0 {
                return Err(invalid("invalid MP4 track time"));
            }
            let multiplier = u64::from(spec.sample_rate)
                .checked_mul(time_base.num as u64)
                .ok_or_else(|| invalid("invalid MP4 track time"))?;
            scale(timestamp as u64, multiplier, time_base.den as u64)
        };
        let mut packets = Vec::with_capacity(samples.len());
        let mut media_stop = 0;
        for (index, sample) in samples.iter().enumerate() {
            let offset = usize::try_from(sample.offset).map_err(invalid)?;
            let size = sample.size as usize;
            let end = offset
                .checked_add(size)
                .ok_or_else(|| invalid("invalid MP4 access unit range"))?;
            if size == 0
                || size > MAX_PACKET_BYTES
                || !guard.contains_media(offset..end)
                || sample.sample_description_index != 1
            {
                return Err(invalid("invalid MP4 AAC access unit"));
            }
            let frame = index * per_packet;
            let stop = sample
                .decode_time
                .checked_add(sample.duration)
                .ok_or_else(|| invalid("invalid MP4 sample timing"))?;
            media_stop = media_frames(stop)?;
            if sample.decode_time != sample.composition_time
                || media_frames(sample.decode_time)? != frame
                || media_stop <= frame
                || media_stop > frame + per_packet
            {
                return Err(unsupported(
                    "MP4 AAC track uses a discontinuous media timeline",
                ));
            }
            packets.push(offset..end);
        }
        let media = match demux.edit_list(stream.index) {
            [] => {
                if let Some(timing) = demux.gapless_timing(stream.index) {
                    let start = usize::try_from(timing.priming).map_err(invalid)?;
                    let stop = timing
                        .priming
                        .checked_add(timing.valid)
                        .and_then(|stop| usize::try_from(stop).ok())
                        .ok_or_else(|| invalid("invalid MP4 gapless timing"))?;
                    start..stop
                } else {
                    0..media_stop
                }
            }
            [edit]
                if edit.media_time >= 0
                    && edit.media_rate_integer == 1
                    && edit.media_rate_fraction == 0 =>
            {
                let start = media_frames(edit.media_time)?;
                let rate = u64::from(demux.movie_timescale());
                let duration = scale(edit.segment_duration, u64::from(spec.sample_rate), rate)?;
                let stop = start
                    .checked_add(duration)
                    .ok_or_else(|| invalid("invalid MP4 edit duration"))?;
                let rounding = scale(1, u64::from(spec.sample_rate), rate)?.max(1);
                if stop > media_stop.saturating_add(rounding) {
                    return Err(invalid("MP4 edit extends outside its audio track"));
                }
                start..stop.min(media_stop)
            }
            _ => {
                return Err(unsupported(
                    "MP4 audio edit lists must have one continuous segment",
                ))
            }
        };
        if media.is_empty() || media.end > media_stop {
            return Err(invalid("MP4 audio presentation bounds are invalid"));
        }
        let mut track = Self {
            bytes,
            packets,
            config,
            spec,
            media,
            decoder: StreamDecoder::new(),
            next_packet: 0,
            pcm: Vec::new(),
        };
        track.reset(0);
        Ok(track)
    }

    fn reset(&mut self, first_packet: usize) {
        self.decoder = StreamDecoder::new();
        self.decoder
            .set_frame_family(self.config.ga_body.frame_length.family(self.config.aot));
        self.next_packet = first_packet;
        self.pcm.clear();
    }

    pub(super) fn samples(
        &mut self,
        begin: u64,
        end: Option<u64>,
    ) -> Result<(WavSpec, Vec<i16>), EngineError> {
        let rate = self.spec.sample_rate;
        let start = frame_at(begin, rate)?;
        let stop = end
            .map(|time| frame_at(time, rate))
            .transpose()?
            .unwrap_or(self.media.len());
        if start >= stop || stop > self.media.len() {
            return Err(invalid(
                "media-overlay audio clip lies outside its MP4 resource",
            ));
        }
        let channels = usize::from(self.spec.channels);
        if stop - start > MAX_CLIP_SAMPLES / channels {
            return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
        }
        let mut samples = Vec::with_capacity((stop - start) * channels);
        let per_packet = self.config.ga_body.frame_length.samples() as usize;
        let start = start + self.media.start;
        let stop = stop + self.media.start;
        let first = start / per_packet;
        let last = (stop - 1) / per_packet;
        // AAC-LC needs preceding overlap samples, so warm up two access
        // units when jumping. Adjacent cues keep the existing decoder state.
        let preroll = first.saturating_sub(2);
        if first < self.next_packet.saturating_sub(1) || preroll > self.next_packet {
            self.reset(preroll);
        }
        if last.saturating_sub(self.next_packet) >= super::MAX_FRAMES_PER_CLIP {
            return Err(limited("MP4 clip exceeds its access unit decoding budget"));
        }
        for index in first..=last {
            while self.next_packet <= index {
                let payload = &self.bytes[self.packets[self.next_packet].clone()];
                let frame = self
                    .decoder
                    .decode_raw_data_block(
                        self.config.aot,
                        self.config.sampling_frequency_index,
                        rate,
                        self.config.channel_configuration,
                        1,
                        payload,
                    )
                    .map_err(invalid)?;
                if frame.channels != channels
                    || frame.sample_rate != rate
                    || frame.pcm.len() != per_packet * channels
                {
                    return Err(unsupported("MP4 track changes its AAC output geometry"));
                }
                self.pcm = frame.pcm;
                self.next_packet += 1;
            }
            let base = index * per_packet;
            let left = start.saturating_sub(base) * channels;
            let right = (stop - base).min(per_packet) * channels;
            samples.extend_from_slice(&self.pcm[left..right]);
        }
        Ok((self.spec, samples))
    }
}

fn scale(value: u64, multiplier: u64, divisor: u64) -> Result<usize, EngineError> {
    if divisor == 0 {
        return Err(invalid("MP4 timescale is zero"));
    }
    let result = (u128::from(value) * u128::from(multiplier) + u128::from(divisor / 2))
        / u128::from(divisor);
    usize::try_from(result).map_err(|_| limited("MP4 audio time exceeds limits"))
}

#[derive(Default)]
struct Mp4Bounds {
    boxes: usize,
    metadata: usize,
    samples: usize,
    media: Vec<Range<usize>>,
}

impl Mp4Bounds {
    fn contains_media(&self, range: Range<usize>) -> bool {
        let index = self.media.partition_point(|mdat| mdat.end <= range.start);
        self.media
            .get(index)
            .is_some_and(|mdat| mdat.start <= range.start && range.end <= mdat.end)
    }

    fn check(&mut self, bytes: &[u8], base: usize, depth: usize) -> Result<(), EngineError> {
        if depth > 8 {
            return Err(limited("MP4 audio box nesting exceeds limits"));
        }
        let mut cursor = Cursor::new(bytes);
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
                .ok_or_else(|| invalid("MP4 audio box extends outside its parent"))?;
            self.boxes += 1;
            if self.boxes > 20_000 {
                return Err(limited("too many MP4 audio boxes"));
            }
            if depth == 0 && header.fourcc == *b"mdat" {
                self.media.push(base + start..base + end);
            } else if depth == 0 {
                self.metadata += length;
                if self.metadata > MAX_METADATA_BYTES {
                    return Err(limited("MP4 audio metadata exceeds 4 MiB"));
                }
            }
            match &header.fourcc {
                b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" => {
                    self.check(&bytes[start..end], base + start, depth + 1)?;
                }
                b"stsz" | b"stz2" => {
                    let raw = bytes[start..end]
                        .get(8..12)
                        .ok_or_else(|| invalid("truncated MP4 sample count"))?;
                    self.samples += u32::from_be_bytes(raw.try_into().map_err(invalid)?) as usize;
                    if self.samples > MAX_ACCESS_UNITS {
                        return Err(limited("too many MP4 audio access units"));
                    }
                }
                b"moof" | b"mvex" => {
                    return Err(unsupported("fragmented MP4 narration is unsupported"))
                }
                _ => {}
            }
            cursor.set_position(end as u64);
        }
        Ok(())
    }
}
