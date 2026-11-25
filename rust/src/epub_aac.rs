//! Bounded AAC-LC decoding on the MP4 presentation timeline.
use std::{io::Cursor, ops::Range, sync::Arc};

use hound::{SampleFormat, WavSpec};
use oxideav_aac::{asc::AudioSpecificConfig, decode::StreamDecoder};
use oxideav_core::{CodecRegistry, Demuxer, MediaType};
use oxideav_mp4::demux::open_typed;

use super::{invalid, limited, unsupported, EngineError, MAX_CLIP_SAMPLES};

use super::super::mp4::{edit_spans, Mp4Bounds, MAX_ACCESS_UNITS};
const MAX_PACKET_BYTES: usize = 32_768;

pub(super) struct Track {
    bytes: Arc<[u8]>,
    packets: Vec<Range<usize>>,
    config: AudioSpecificConfig,
    spec: WavSpec,
    media: Vec<(usize, Range<usize>)>,
    active_span: Option<usize>,
    presentation_frames: usize,
    decoder: StreamDecoder,
    next_packet: usize,
    pcm: Vec<i16>,
}

impl Track {
    pub(super) fn open(bytes: Vec<u8>) -> Result<Self, EngineError> {
        let guard = Mp4Bounds::inspect(&bytes)?;
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
        let movie = demux.movie_header();
        let rate = u64::from(movie.timescale);
        let sample_rate = u64::from(spec.sample_rate);
        // Use the same half-open sample boundary as the public clip clock.
        let movie_frames = |tick: u64| {
            if rate == 0 {
                return Err(invalid("MP4 timescale is zero"));
            }
            usize::try_from((u128::from(tick) * u128::from(sample_rate)).div_ceil(u128::from(rate)))
                .map_err(|_| limited("MP4 audio time exceeds limits"))
        };
        let edits = edit_spans(
            demux.edit_list(stream.index),
            movie.timescale,
            demux
                .track_header(stream.index)
                .map_or(0, |track| track.media_timescale),
            "MP4 audio edit lists must have one continuous segment",
        )?;
        let tail = edits
            .last()
            .map(|edit| edit.implicit_tail_end(movie))
            .transpose()?
            .flatten();
        let mut media = Vec::new();
        if edits.is_empty() {
            let range = if let Some(timing) = demux.gapless_timing(stream.index) {
                let start = usize::try_from(timing.priming).map_err(invalid)?;
                let stop = timing
                    .priming
                    .checked_add(timing.valid)
                    .and_then(|stop| usize::try_from(stop).ok())
                    .ok_or_else(|| invalid("invalid MP4 gapless timing"))?;
                start..stop
            } else {
                0..media_stop
            };
            media.push((0, range));
        }
        for edit in &edits {
            let source_start = media_frames(edit.media_start)?;
            let rounding = scale(1, sample_rate, rate)?.max(1);
            if edits.len() > 1
                && (!(u128::from(edit.movie_start) * u128::from(sample_rate))
                    .is_multiple_of(u128::from(rate))
                    || !(u128::from(edit.movie_end) * u128::from(sample_rate))
                        .is_multiple_of(u128::from(rate))
                    || !(u128::from(edit.media_start as u64)
                        * u128::from(sample_rate)
                        * time_base.num as u128)
                        .is_multiple_of(time_base.den as u128))
            {
                return Err(unsupported(
                    "MP4 AAC discontinuous edits require sample-aligned boundaries",
                ));
            }
            let (start, duration) = if edit.movie_start == 0 && tail.is_none() && edits.len() == 1 {
                (0, scale(edit.movie_end, sample_rate, rate)?)
            } else {
                let start = movie_frames(edit.movie_start)?;
                (start, movie_frames(edit.movie_end)? - start)
            };
            let stop = source_start
                .checked_add(duration)
                .ok_or_else(|| invalid("invalid MP4 edit duration"))?;
            if stop > media_stop.saturating_add(rounding) || (edits.len() > 1 && stop > media_stop)
            {
                return Err(invalid("MP4 edit extends outside its audio track"));
            }
            media.push((start, source_start..stop.min(media_stop)));
        }
        if media
            .iter()
            .any(|(_, range)| range.is_empty() || range.end > media_stop)
        {
            return Err(invalid("MP4 audio presentation bounds are invalid"));
        }
        let (last_start, last_media) = media
            .last()
            .ok_or_else(|| invalid("missing MP4 audio interval"))?;
        let active_end = last_start
            .checked_add(last_media.len())
            .ok_or_else(|| limited("MP4 audio time exceeds limits"))?;
        let presentation_frames = tail.map(movie_frames).transpose()?.unwrap_or(active_end);
        let mut track = Self {
            bytes,
            packets,
            config,
            spec,
            media,
            active_span: None,
            presentation_frames,
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

    pub(super) fn spec(&self) -> WavSpec {
        self.spec
    }

    pub(super) fn frames(&self) -> usize {
        self.presentation_frames
    }

    pub(super) fn samples(&mut self, start: usize, stop: usize) -> Result<Vec<i16>, EngineError> {
        let channels = usize::from(self.spec.channels);
        if stop - start > MAX_CLIP_SAMPLES / channels {
            return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
        }
        let mut samples = Vec::with_capacity((stop - start) * channels);
        let mut packets_left = super::MAX_FRAMES_PER_CLIP;
        for index in 0..self.media.len() {
            let (movie_start, range) = &self.media[index];
            let left = start.max(*movie_start);
            let right = stop.min(*movie_start + range.len());
            if left < right {
                let source_start = left - *movie_start + range.start;
                let source_end = right - *movie_start + range.start;
                samples.resize((left - start) * channels, 0);
                let reset = self.active_span != Some(index);
                self.append_media(
                    source_start,
                    source_end,
                    reset,
                    &mut packets_left,
                    &mut samples,
                )?;
                self.active_span = Some(index);
            }
        }
        samples.resize((stop - start) * channels, 0);
        Ok(samples)
    }

    fn append_media(
        &mut self,
        start: usize,
        stop: usize,
        reset: bool,
        packets_left: &mut usize,
        samples: &mut Vec<i16>,
    ) -> Result<(), EngineError> {
        let rate = self.spec.sample_rate;
        let channels = usize::from(self.spec.channels);
        let per_packet = self.config.ga_body.frame_length.samples() as usize;
        let first = start / per_packet;
        let last = (stop - 1) / per_packet;
        // AAC-LC needs preceding overlap samples, so warm up two access
        // units when jumping. Adjacent cues keep the existing decoder state.
        let preroll = first.saturating_sub(2);
        if reset || first < self.next_packet.saturating_sub(1) || preroll > self.next_packet {
            self.reset(preroll);
        }
        *packets_left = packets_left
            .checked_sub((last + 1).saturating_sub(self.next_packet))
            .ok_or_else(|| limited("MP4 clip exceeds its access unit decoding budget"))?;
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
        Ok(())
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
