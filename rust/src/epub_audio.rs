//! Decode bounded sections of local media-overlay audio for an offline playback sink.
use std::{io::Cursor, ops::Range, panic::AssertUnwindSafe};

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use super::{
    invalid, limited,
    media::{audio_codec, AudioCodec},
    ogg::{self, Codec},
    parse_xml, unsupported, validate_archive_path, Book, XHTML, XML_BYTES,
};
use crate::EngineError;

#[path = "epub_aac.rs"]
mod aac;
#[path = "epub_ogg_mixed.rs"]
pub(in crate::epub) mod ogg_mixed;
#[path = "epub_opus.rs"]
pub(super) mod opus;
#[path = "epub_vorbis.rs"]
pub(in crate::epub) mod vorbis;

pub(super) const MAX_ENCODED_BYTES: usize = 32 * 1024 * 1024;
const MAX_CLIP_SAMPLES: usize = 8 * 1024 * 1024;
const MAX_FRAMES_PER_CLIP: usize = 120_000;
const MP3_DECODER_DELAY: usize = 529;
const MAX_NARRATION_BYTES: usize = 1024 * 1024;

// Vorbis I §4.3.9 and Opus channel mapping family 1 share this order.
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

pub(crate) struct AudioSource {
    decoder: Decoder,
    stream: Option<AudioWindow>,
}

struct AudioWindow {
    spec: WavSpec,
    speaker_mask: Option<u32>,
    next: usize,
    end: Option<usize>,
    allow_eof: bool,
    started: bool,
}

enum Decoder {
    Wave(WavReader<Cursor<Vec<u8>>>),
    Mp3(Box<Mp3>),
    Aac(Box<aac::Track>),
    Opus(Box<opus::Track>),
    Vorbis(Box<vorbis::Track>),
    MixedOgg(Box<ogg_mixed::Track>),
}

pub(crate) struct AudioClip {
    pub wave: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: usize,
}

impl AudioSource {
    pub(crate) fn open(
        data: &[u8],
        path: &str,
        media: &str,
        opus_track: Option<usize>,
        synchronize_video: bool,
        audio_track: Option<usize>,
        names: &[String],
    ) -> Result<Self, EngineError> {
        Self::decode(
            media,
            opus_track,
            synchronize_video,
            audio_track,
            names,
            || read(data, path, MAX_ENCODED_BYTES),
        )
    }

    fn decode(
        media: &str,
        opus_track: Option<usize>,
        synchronize_video: bool,
        audio_track: Option<usize>,
        names: &[String],
        load: impl FnOnce() -> Result<Vec<u8>, EngineError>,
    ) -> Result<Self, EngineError> {
        let codec = audio_codec(media);
        if !names.is_empty() && (opus_track.is_some() || audio_track.is_some()) {
            return Err(EngineError::InvalidArgument(
                "named media fragments cannot be combined with an audio track ordinal".into(),
            ));
        }
        if !names.is_empty()
            && !matches!(
                codec,
                Some(AudioCodec::Ogg | AudioCodec::Opus | AudioCodec::Vorbis | AudioCodec::Webm)
            )
        {
            return Err(unsupported(
                "named media fragments require local WebM or Ogg",
            ));
        }
        if opus_track.is_some() && audio_track.is_some() {
            return Err(EngineError::InvalidArgument(
                "Opus and WebM audio track selections are mutually exclusive".into(),
            ));
        }
        if opus_track.is_some() && !matches!(codec, Some(AudioCodec::Opus | AudioCodec::Ogg)) {
            return Err(EngineError::InvalidArgument(
                "Opus track selection requires an Ogg audio resource".into(),
            ));
        }
        if audio_track.is_some() && !matches!(codec, Some(AudioCodec::Webm)) {
            return Err(EngineError::InvalidArgument(
                "WebM audio track selection requires a WebM resource".into(),
            ));
        }
        let decoder = match codec {
            Some(AudioCodec::Ogg | AudioCodec::Opus | AudioCodec::Vorbis) => {
                let bytes = load()?;
                std::panic::catch_unwind(|| {
                    let named = (!names.is_empty())
                        .then(|| ogg::named_tracks(&bytes, names))
                        .transpose()?;
                    if let Some(tracks) = named.as_ref().filter(|tracks| tracks.mixed_audio()) {
                        let mut audio = ogg_mixed::Track::open(&bytes, tracks)?;
                        if synchronize_video {
                            super::video::align_mixed_audio(&bytes, &mut audio, tracks)?;
                        }
                        return Ok(Decoder::MixedOgg(Box::new(audio)));
                    }
                    let codec = match (named.as_ref(), codec) {
                        (Some(tracks), _) => tracks.audio_codec().ok_or_else(|| {
                            unsupported("named Ogg fragment does not select an audio track")
                        })?,
                        (_, Some(AudioCodec::Vorbis)) => Codec::Vorbis,
                        (_, Some(AudioCodec::Opus)) => Codec::Opus,
                        _ if opus_track.is_some() => Codec::Opus,
                        _ => ogg::audio_codec(&bytes)?,
                    };
                    if codec == Codec::Vorbis {
                        let mut track = vorbis::Track::open_ogg(&bytes, named.as_ref())?;
                        if synchronize_video {
                            super::video::align_vorbis_audio(&bytes, &mut track, named.as_ref())?;
                        }
                        Ok(Decoder::Vorbis(Box::new(track)))
                    } else {
                        let mut track = opus::Track::open(&bytes, opus_track, named.as_ref())?;
                        if synchronize_video {
                            super::video::align_audio(&bytes, &mut track, named.as_ref())?;
                        }
                        Ok(Decoder::Opus(Box::new(track)))
                    }
                })
                .map_err(|_| invalid("malformed Ogg audio"))??
            }
            Some(AudioCodec::Webm) => {
                let bytes = load()?;
                std::panic::catch_unwind(|| {
                    let reader = super::webm::Reader::open(bytes)?;
                    let index = reader.track(oxideav_core::MediaType::Audio, audio_track, names)?;
                    use oxideav_core::Demuxer;
                    match reader.demux.streams()[index as usize]
                        .params
                        .codec_id
                        .as_str()
                    {
                        "opus" => Ok(Decoder::Opus(Box::new(opus::Track::open_webm(
                            reader, index,
                        )?))),
                        "vorbis" => Ok(Decoder::Vorbis(Box::new(vorbis::Track::open(
                            reader, index,
                        )?))),
                        _ => Err(unsupported("WebM audio requires an Opus or Vorbis track")),
                    }
                })
                .map_err(|_| invalid("malformed WebM audio"))??
            }
            Some(AudioCodec::Mpeg) => {
                let bytes = load()?;
                let track = std::panic::catch_unwind(|| Mp3::new(bytes))
                    .map_err(|_| invalid("malformed MPEG Layer III audio"))??;
                Decoder::Mp3(Box::new(track))
            }
            Some(AudioCodec::Aac) => {
                let bytes = load()?;
                let track = std::panic::catch_unwind(|| aac::Track::open(bytes))
                    .map_err(|_| invalid("malformed MP4 audio"))??;
                Decoder::Aac(Box::new(track))
            }
            Some(AudioCodec::Wave) => {
                let bytes = load()?;
                Decoder::Wave(WavReader::new(Cursor::new(bytes)).map_err(wav_error)?)
            }
            None => return Err(unsupported("EPUB audio codec is unsupported")),
        };
        Ok(Self {
            decoder,
            stream: None,
        })
    }

    pub(super) fn presentation_end(
        media: &str,
        load: impl FnOnce() -> Result<Vec<u8>, EngineError>,
    ) -> Result<u64, EngineError> {
        let mut source = Self::decode(media, None, false, None, &[], load)?;
        let (format, frames) = source.geometry()?;
        let frames = match (&source.decoder, frames) {
            (_, Some(frames)) => frames,
            (Decoder::Mp3(track), None) => std::panic::catch_unwind(|| track.scan_frames(format))
                .map_err(|_| invalid("malformed MPEG Layer III audio"))??,
            _ => return Err(invalid("missing media-overlay audio duration")),
        };
        let micros = (frames as u128 * 1_000_000) / u128::from(format.sample_rate);
        u64::try_from(micros).map_err(|_| limited("media-overlay audio duration exceeds limits"))
    }

    pub(crate) fn clip(&mut self, begin: u64, end: Option<u64>) -> Result<AudioClip, EngineError> {
        self.stream = None;
        let window = self.window(begin, end, false)?;
        let samples = self.read_frames(window.next, window.end, window.end.is_none())?;
        encode_clip(window.spec, window.speaker_mask, samples)
    }

    pub(crate) fn start_stream(
        &mut self,
        begin: u64,
        end: Option<u64>,
        clamp_to_duration: bool,
    ) -> Result<(), EngineError> {
        self.stream = None;
        self.stream = Some(self.window(begin, end, clamp_to_duration)?);
        Ok(())
    }

    pub(crate) fn next_chunk(
        &mut self,
        chunk_frames: usize,
    ) -> Result<Option<AudioClip>, EngineError> {
        if !(1..=262_144).contains(&chunk_frames) {
            return Err(EngineError::InvalidArgument(
                "EPUB narration chunk_frames must be between 1 and 262144".into(),
            ));
        }
        let Some(mut window) = self.stream.take() else {
            return Ok(None);
        };
        let limit = window
            .next
            .checked_add(chunk_frames)
            .ok_or_else(|| limited("media-overlay audio time exceeds limits"))?;
        let stop = window.end.map_or(limit, |end| end.min(limit));
        let samples = self.read_frames(window.next, Some(stop), window.allow_eof)?;
        if samples.is_empty() && window.started && window.allow_eof {
            return Ok(None);
        }
        let result = encode_clip(window.spec, window.speaker_mask, samples)?;
        window.next += result.frames;
        window.started = true;
        // When a decoder returns fewer frames than requested, it reached EOF.
        if window.next == stop && window.end != Some(stop) {
            self.stream = Some(window);
        }
        Ok(Some(result))
    }

    fn window(
        &mut self,
        begin: u64,
        end: Option<u64>,
        clamp_to_duration: bool,
    ) -> Result<AudioWindow, EngineError> {
        if end.is_some_and(|end| end <= begin) {
            return Err(invalid("media-overlay audio end must follow its start"));
        }
        let (spec, duration) = self.geometry()?;
        let next = frame_at(begin, spec.sample_rate)?;
        let mut end = end
            .map(|end| frame_at(end, spec.sample_rate))
            .transpose()?
            .or(duration);
        if clamp_to_duration {
            end = end.into_iter().chain(duration).min();
        }
        if end.is_some_and(|end| end <= next || duration.is_some_and(|duration| end > duration)) {
            return Err(invalid(
                "media-overlay audio clip lies outside its resource or contains no sample frames",
            ));
        }
        let speaker_mask = match &self.decoder {
            Decoder::Opus(track) => track.speaker_mask(),
            Decoder::Vorbis(track) => track.speaker_mask(),
            Decoder::MixedOgg(track) => track.speaker_mask(),
            _ => None,
        };
        Ok(AudioWindow {
            spec,
            speaker_mask,
            next,
            end,
            allow_eof: end.is_none() || (clamp_to_duration && duration.is_none()),
            started: false,
        })
    }

    fn geometry(&mut self) -> Result<(WavSpec, Option<usize>), EngineError> {
        Ok(match &mut self.decoder {
            Decoder::Wave(reader) => (wav_spec(reader)?, Some(reader.duration() as usize)),
            Decoder::Aac(track) => (track.spec(), Some(track.frames())),
            Decoder::Opus(track) => (track.spec(), Some(track.frames())),
            Decoder::Vorbis(track) => (track.spec(), Some(track.frames())),
            Decoder::MixedOgg(track) => (track.spec(), Some(track.frames())),
            Decoder::Mp3(reader) => {
                std::panic::catch_unwind(AssertUnwindSafe(|| reader.geometry()))
                    .map_err(|_| invalid("malformed MPEG Layer III audio"))??
            }
        })
    }

    fn read_frames(
        &mut self,
        start: usize,
        end: Option<usize>,
        allow_eof: bool,
    ) -> Result<Vec<i16>, EngineError> {
        let known_end = || end.ok_or_else(|| invalid("missing media-overlay audio duration"));
        match &mut self.decoder {
            Decoder::Wave(reader) => wav_samples(reader, start, known_end()?),
            Decoder::Aac(track) => {
                let end = known_end()?;
                std::panic::catch_unwind(AssertUnwindSafe(|| track.samples(start, end)))
                    .map_err(|_| invalid("malformed MP4 audio"))?
            }
            Decoder::Opus(track) => {
                let end = known_end()?;
                std::panic::catch_unwind(AssertUnwindSafe(|| track.samples(start, end)))
                    .map_err(|_| invalid("malformed Opus audio"))?
            }
            Decoder::Vorbis(track) => {
                let end = known_end()?;
                std::panic::catch_unwind(AssertUnwindSafe(|| track.samples(start, end)))
                    .map_err(|_| invalid("malformed Vorbis audio"))?
            }
            Decoder::MixedOgg(track) => {
                let end = known_end()?;
                std::panic::catch_unwind(AssertUnwindSafe(|| track.samples(start, end)))
                    .map_err(|_| invalid("malformed mixed Ogg audio"))?
            }
            Decoder::Mp3(reader) => {
                std::panic::catch_unwind(AssertUnwindSafe(|| reader.samples(start, end, allow_eof)))
                    .map_err(|_| invalid("malformed MPEG Layer III audio"))?
            }
        }
    }
}

fn encode_clip(
    spec: WavSpec,
    speaker_mask: Option<u32>,
    samples: Vec<i16>,
) -> Result<AudioClip, EngineError> {
    let frames = samples.len() / usize::from(spec.channels);
    if frames == 0 {
        return Err(invalid("media-overlay audio clip has no samples"));
    }
    let mut output = Cursor::new(Vec::new());
    let mut writer = WavWriter::new(
        &mut output,
        WavSpec {
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
            ..spec
        },
    )
    .map_err(wav_error)?;
    let mut writer_samples = writer.get_i16_writer(samples.len() as u32);
    for sample in samples {
        writer_samples.write_sample(sample);
    }
    writer_samples.flush().map_err(wav_error)?;
    writer.finalize().map_err(wav_error)?;
    let mut wave = output.into_inner();
    if let Some(mask) = speaker_mask {
        // Hound writes a 40-byte extensible fmt chunk for more than two channels,
        // but its default mask cannot describe layouts such as quad or 7.1.
        if wave.get(12..22) != Some(b"fmt \x28\0\0\0\xfe\xff") {
            return Err(invalid("missing extensible WAVE format for surround audio"));
        }
        let slot = wave
            .get_mut(40..44)
            .ok_or_else(|| invalid("missing WAVE speaker mask"))?;
        slot.copy_from_slice(&mask.to_le_bytes());
    }
    Ok(AudioClip {
        wave,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        frames,
    })
}

fn read(data: &[u8], path: &str, limit: usize) -> Result<Vec<u8>, EngineError> {
    validate_archive_path(path)?;
    Book::open(data)?.read(path, limit)
}

fn frame_at(microseconds: u64, rate: u32) -> Result<usize, EngineError> {
    let frame = (u128::from(microseconds) * u128::from(rate)).div_ceil(1_000_000);
    usize::try_from(frame).map_err(|_| limited("media-overlay audio time exceeds limits"))
}

fn wav_spec(reader: &WavReader<Cursor<Vec<u8>>>) -> Result<WavSpec, EngineError> {
    let spec = reader.spec();
    if spec.channels == 0 || spec.channels > 8 || !(1..=192_000).contains(&spec.sample_rate) {
        return Err(unsupported(
            "WAVE sample rate or channel count is unsupported",
        ));
    }
    Ok(spec)
}

fn wav_samples(
    reader: &mut WavReader<Cursor<Vec<u8>>>,
    start: usize,
    stop: usize,
) -> Result<Vec<i16>, EngineError> {
    let spec = reader.spec();
    let count = (stop - start).saturating_mul(usize::from(spec.channels));
    if count > MAX_CLIP_SAMPLES {
        return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
    }
    reader.seek(start as u32).map_err(invalid)?;
    let mut samples = Vec::with_capacity(count);
    match spec.sample_format {
        SampleFormat::Int => {
            for raw in reader.samples::<i32>().take(count) {
                let value = raw.map_err(wav_error)?;
                let shift = i32::from(spec.bits_per_sample) - 16;
                samples.push(if shift >= 0 {
                    (value >> shift) as i16
                } else {
                    (value << -shift) as i16
                });
            }
        }
        SampleFormat::Float => {
            for raw in reader.samples::<f32>().take(count) {
                let value = raw.map_err(wav_error)?;
                if !value.is_finite() {
                    return Err(invalid("non-finite WAVE sample"));
                }
                samples.push((value.clamp(-1.0, 1.0) * 32768.0).round() as i16);
            }
        }
    }
    if samples.len() != count {
        return Err(invalid("truncated WAVE samples"));
    }
    Ok(samples)
}

fn wav_error(error: hound::Error) -> EngineError {
    match error {
        hound::Error::Unsupported => unsupported("unsupported WAVE sample format"),
        _ => invalid(error),
    }
}

struct Mp3 {
    bytes: Vec<u8>,
    first: usize,
    byte_offset: usize,
    decoder: mp3dec::Decoder,
    pcm: [i16; mp3dec::MAX_SAMPLES_PER_FRAME],
    sample_offset: usize,
    frame_len: usize,
    format: Option<WavSpec>,
    presentation: Option<Range<usize>>,
}

impl Mp3 {
    fn new(mut bytes: Vec<u8>) -> Result<Self, EngineError> {
        if bytes.len() >= 128 && bytes[bytes.len() - 128..].starts_with(b"TAG") {
            bytes.truncate(bytes.len() - 128);
        }
        let mut first = 0;
        if bytes.starts_with(b"ID3") {
            let header = bytes
                .get(..10)
                .ok_or_else(|| invalid("truncated ID3 header"))?;
            let mut size = 0;
            for &number in &header[6..10] {
                if number >= 128 {
                    return Err(invalid("invalid ID3 header size"));
                }
                size = (size << 7) | usize::from(number);
            }
            first = 10
                + size
                + if header[3] == 4 && header[5] & 0x10 != 0 {
                    10
                } else {
                    0
                };
        }
        if first >= bytes.len() {
            return Err(invalid("empty MPEG audio stream"));
        }
        let presentation = mpeg_presentation(&bytes[first..])?;
        let (first, presentation) = match presentation {
            Some((metadata_frame, interval)) => (first + metadata_frame, Some(interval)),
            None => (first, None),
        };
        Ok(Self {
            bytes,
            first,
            byte_offset: first,
            decoder: mp3dec::Decoder::new(),
            pcm: [0; mp3dec::MAX_SAMPLES_PER_FRAME],
            sample_offset: 0,
            frame_len: 0,
            format: None,
            presentation,
        })
    }

    fn next_frame(&mut self) -> Result<bool, EngineError> {
        if self.byte_offset == self.bytes.len() {
            return Ok(false);
        }
        let mut info = mp3dec::FrameInfo::default();
        let frames = self.decoder.decode_frame(
            &self.bytes[self.byte_offset..],
            Some(&mut self.pcm),
            &mut info,
        );
        if matches!(info.layer, 1 | 2) {
            return Err(unsupported("MPEG audio requires Layer III"));
        }
        if frames == 0
            || info.frame_offset != 0
            || !matches!(info.channels, 1 | 2)
            || !(8_000..=48_000).contains(&info.hz)
            || frames.saturating_mul(info.channels) > self.pcm.len()
            || info.frame_bytes == 0
            || info.frame_bytes > self.bytes.len() - self.byte_offset
        {
            return Err(invalid("invalid MPEG Layer III frame"));
        }
        let format = WavSpec {
            channels: info.channels as u16,
            sample_rate: info.hz as u32,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        if self.format.is_some_and(|prior| prior != format) {
            return Err(unsupported(
                "MPEG audio changes sample rate or channel count",
            ));
        }
        self.format = Some(format);
        self.sample_offset += self.frame_len;
        self.frame_len = frames;
        self.byte_offset += info.frame_bytes;
        Ok(true)
    }

    fn geometry(&mut self) -> Result<(WavSpec, Option<usize>), EngineError> {
        if self.format.is_none() {
            self.next_frame()?;
        }
        self.format
            .map(|format| (format, self.presentation.as_ref().map(Range::len)))
            .ok_or_else(|| invalid("empty MPEG audio stream"))
    }

    fn scan_frames(&self, format: WavSpec) -> Result<usize, EngineError> {
        let mut scanner = mp3dec::Decoder::new();
        let mut bytes = &self.bytes[self.first..];
        let mut frames = 0;
        for _ in 0..MAX_FRAMES_PER_CLIP {
            let mut info = mp3dec::FrameInfo::default();
            let count = scanner.decode_frame(bytes, None, &mut info);
            if count == 0
                || info.frame_offset != 0
                || info.layer != 3
                || info.hz != format.sample_rate as usize
                || info.channels != usize::from(format.channels)
                || info.frame_bytes == 0
            {
                return Err(invalid(
                    "invalid MPEG Layer III frame in media-overlay audio",
                ));
            }
            bytes = bytes
                .get(info.frame_bytes..)
                .ok_or_else(|| invalid("invalid MPEG audio frame size"))?;
            frames += count;
            if bytes.is_empty() {
                return Ok(frames);
            }
        }
        Err(limited("MPEG audio duration exceeds its frame budget"))
    }

    fn samples(
        &mut self,
        start: usize,
        stop: Option<usize>,
        allow_eof: bool,
    ) -> Result<Vec<i16>, EngineError> {
        let (format, _) = self.geometry()?;
        let origin = self
            .presentation
            .as_ref()
            .map_or(0, |interval| interval.start);
        let start = start + origin;
        let stop = stop.map(|frame| frame + origin);
        let channels = usize::from(format.channels);
        if stop.is_some_and(|stop| stop.saturating_sub(start) > MAX_CLIP_SAMPLES / channels) {
            return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
        }
        if start < self.sample_offset {
            self.byte_offset = self.first;
            self.decoder = mp3dec::Decoder::new();
            self.sample_offset = 0;
            self.frame_len = 0;
            self.next_frame()?;
        }
        let mut samples = Vec::new();
        for _ in 0..MAX_FRAMES_PER_CLIP {
            let frame_end = self.sample_offset + self.frame_len;
            let left = start.max(self.sample_offset);
            let right = stop.unwrap_or(frame_end).min(frame_end);
            if left < right {
                let slice =
                    (left - self.sample_offset) * channels..(right - self.sample_offset) * channels;
                if samples.len() + slice.len() > MAX_CLIP_SAMPLES {
                    return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
                }
                samples.extend_from_slice(&self.pcm[slice]);
            }
            if stop.is_some_and(|stop| stop <= frame_end) {
                return Ok(samples);
            }
            if !self.next_frame()? {
                if !allow_eof {
                    return Err(invalid(
                        "media-overlay audio clip lies outside its MPEG resource",
                    ));
                }
                return Ok(samples);
            }
        }
        Err(limited("MPEG clip exceeds its frame decoding budget"))
    }
}

/// Parse encoder delay and padding from a Xing/Info frame. Packet headers
/// confirm the declared sample count before using it as a presentation bound.
fn mpeg_presentation(bytes: &[u8]) -> Result<Option<(usize, Range<usize>)>, EngineError> {
    let Some(header) = bytes.get(..4) else {
        return Ok(None);
    };
    // The Xing signature follows the MPEG header and version/channel-dependent
    // Layer III side information. A LAME-style CRC does not move the signature.
    if header[0] != 0xff || header[1] & 0xe6 != 0xe2 {
        return Ok(None);
    }
    let mpeg1 = header[1] & 0x18 == 0x18;
    let mono = header[3] >> 6 == 3;
    let offset = 4 + match (mpeg1, mono) {
        (true, false) => 32,
        (true, true) | (false, false) => 17,
        (false, true) => 9,
    };
    if !matches!(bytes.get(offset..offset + 4), Some(b"Xing" | b"Info")) {
        return Ok(None);
    }

    let mut scanner = mp3dec::Decoder::new();
    let mut info = mp3dec::FrameInfo::default();
    let samples = scanner.decode_frame(bytes, None, &mut info);
    if samples == 0 || info.layer != 3 || info.frame_offset != 0 {
        return Err(invalid("invalid MPEG metadata frame"));
    }
    let mut fields = bytes
        .get(offset + 4..info.frame_bytes)
        .ok_or_else(|| invalid("truncated MPEG metadata frame"))?;
    let field = |fields: &mut &[u8]| -> Result<usize, EngineError> {
        let (value, rest) = fields
            .split_first_chunk::<4>()
            .ok_or_else(|| invalid("truncated MPEG metadata field"))?;
        *fields = rest;
        Ok(u32::from_be_bytes(*value) as usize)
    };
    let flags = field(&mut fields)?;
    let count = if flags & 1 != 0 {
        field(&mut fields)?
    } else {
        0
    };
    if flags & 2 != 0 {
        let _ = field(&mut fields)?;
    }
    if flags & 4 != 0 {
        fields = fields
            .get(100..)
            .ok_or_else(|| invalid("truncated MPEG seek table"))?;
    }
    if flags & 8 != 0 {
        let _ = field(&mut fields)?;
    }
    if !matches!(fields.get(..4), Some(b"LAME" | b"Lavc" | b"Lavf")) || count == 0 {
        return Ok(None);
    }
    let delay = fields
        .get(21..24)
        .ok_or_else(|| invalid("truncated MPEG encoder padding"))?;
    let delay_and_padding =
        (usize::from(delay[0]) << 16) | (usize::from(delay[1]) << 8) | usize::from(delay[2]);
    let encoder_delay = delay_and_padding >> 12;
    let padding = delay_and_padding & 0xfff;
    if encoder_delay == 0 && padding == 0 {
        return Ok(None);
    }
    if padding < MP3_DECODER_DELAY {
        return Err(unsupported(
            "MPEG encoder padding cannot represent the decoder delay",
        ));
    }
    if count > 250_000 {
        return Err(limited("MPEG audio exceeds its packet count limit"));
    }
    let first = info.frame_bytes;
    let mut remaining = bytes
        .get(first..)
        .ok_or_else(|| invalid("invalid MPEG metadata size"))?;
    let geometry = (info.hz, info.channels);
    for _ in 0..count {
        let mut frame = mp3dec::FrameInfo::default();
        let length = scanner.decode_frame(remaining, None, &mut frame);
        if length != samples
            || frame.frame_offset != 0
            || frame.layer != 3
            || (frame.hz, frame.channels) != geometry
        {
            return Err(invalid(
                "MPEG gapless metadata does not match its audio frames",
            ));
        }
        remaining = remaining
            .get(frame.frame_bytes..)
            .ok_or_else(|| invalid("invalid MPEG audio frame size"))?;
    }
    if !remaining.is_empty() {
        return Err(invalid("MPEG gapless frame count does not match its audio"));
    }
    let begin = encoder_delay + MP3_DECODER_DELAY;
    let end = (count * samples)
        .checked_sub(padding - MP3_DECODER_DELAY)
        .filter(|end| *end > begin)
        .ok_or_else(|| invalid("MPEG encoder padding leaves no playable samples"))?;
    Ok(Some((first, begin..end)))
}

pub(crate) fn narration_text(
    data: &[u8],
    path: &str,
    fragment: Option<&str>,
) -> Result<String, EngineError> {
    let source = read(data, path, XML_BYTES)?;
    let doc = parse_xml(&source)?;
    let root = doc.root_element();
    let target = match fragment {
        Some(id) => root
            .descendants()
            .find(|node| super::element_ids(*node).any(|value| value == id)),
        None if super::name(root, XHTML, "html") => super::child(root, XHTML, "body"),
        None => Some(root),
    }
    .ok_or_else(|| invalid("media-overlay narration target is missing"))?;
    let mut text = String::new();
    collect_text(target, &mut text)?;
    Ok(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn collect_text(node: roxmltree::Node<'_, '_>, out: &mut String) -> Result<(), EngineError> {
    let name = node.tag_name().name();
    if node.is_element()
        && (node.has_attribute("hidden")
            || matches!(name, "script" | "style" | "template" | "head"))
    {
        return Ok(());
    }
    let block = matches!(
        name,
        "br" | "div"
            | "p"
            | "blockquote"
            | "li"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "section"
            | "td"
            | "th"
    );
    if block {
        append_text(out, " ")?;
    }
    if node.is_text() {
        append_text(out, node.text().unwrap_or_default())?;
    } else if name == "img" {
        append_text(out, node.attribute("alt").unwrap_or_default())?;
    }
    for child in node.children() {
        collect_text(child, out)?;
    }
    if block {
        append_text(out, " ")?;
    }
    Ok(())
}

fn append_text(output: &mut String, part: &str) -> Result<(), EngineError> {
    if part.len() > MAX_NARRATION_BYTES - output.len() {
        return Err(limited("media-overlay narration text exceeds 1 MiB"));
    }
    output.push_str(part);
    Ok(())
}
