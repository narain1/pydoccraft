//! Decode bounded sections of local media-overlay audio for an offline playback sink.
use std::{io::Cursor, panic::AssertUnwindSafe};

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use super::{
    invalid, limited, parse_xml, unsupported, validate_archive_path, Book, XHTML, XML_BYTES,
};
use crate::EngineError;

#[path = "epub_aac.rs"]
mod aac;

const MAX_ENCODED_BYTES: usize = 32 * 1024 * 1024;
const MAX_CLIP_SAMPLES: usize = 8 * 1024 * 1024;
const MAX_FRAMES_PER_CLIP: usize = 120_000;
const MAX_NARRATION_BYTES: usize = 1024 * 1024;

pub(crate) struct AudioSource(Decoder);

enum Decoder {
    Wave(WavReader<Cursor<Vec<u8>>>),
    Mp3(Box<Mp3>),
    Aac(Box<aac::Track>),
}

pub(crate) struct AudioClip {
    pub wave: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: usize,
}

impl AudioSource {
    pub(crate) fn open(data: &[u8], path: &str, media: &str) -> Result<Self, EngineError> {
        let decoder = match media {
            "audio/mpeg" | "audio/mp3" => {
                let bytes = read(data, path, MAX_ENCODED_BYTES)?;
                Decoder::Mp3(Box::new(Mp3::new(bytes)?))
            }
            "audio/mp4" | "audio/x-m4a" => {
                let bytes = read(data, path, MAX_ENCODED_BYTES)?;
                let track = std::panic::catch_unwind(|| aac::Track::open(bytes))
                    .map_err(|_| invalid("malformed MP4 audio"))??;
                Decoder::Aac(Box::new(track))
            }
            "audio/wav" | "audio/wave" | "audio/x-wav" | "audio/vnd.wave" => {
                let bytes = read(data, path, MAX_ENCODED_BYTES)?;
                Decoder::Wave(WavReader::new(Cursor::new(bytes)).map_err(wav_error)?)
            }
            _ => return Err(unsupported("media-overlay audio codec is unsupported")),
        };
        Ok(Self(decoder))
    }

    pub(crate) fn clip(&mut self, begin: u64, end: Option<u64>) -> Result<AudioClip, EngineError> {
        if end.is_some_and(|end| end <= begin) {
            return Err(invalid("media-overlay audio end must follow its start"));
        }
        let (spec, samples) = match &mut self.0 {
            Decoder::Wave(reader) => wav_samples(reader, begin, end)?,
            Decoder::Aac(track) => {
                std::panic::catch_unwind(AssertUnwindSafe(|| track.samples(begin, end)))
                    .map_err(|_| invalid("malformed MP4 audio"))??
            }
            Decoder::Mp3(reader) => {
                std::panic::catch_unwind(AssertUnwindSafe(|| reader.samples(begin, end)))
                    .map_err(|_| invalid("malformed MPEG Layer III audio"))??
            }
        };
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
        Ok(AudioClip {
            wave: output.into_inner(),
            sample_rate: spec.sample_rate,
            channels: spec.channels,
            frames,
        })
    }
}

fn read(data: &[u8], path: &str, limit: usize) -> Result<Vec<u8>, EngineError> {
    validate_archive_path(path)?;
    Book::open(data)?.read(path, limit)
}

fn frame_at(microseconds: u64, rate: u32) -> Result<usize, EngineError> {
    let frame = (u128::from(microseconds) * u128::from(rate)).div_ceil(1_000_000);
    usize::try_from(frame).map_err(|_| limited("media-overlay audio time exceeds limits"))
}

fn wav_samples(
    reader: &mut WavReader<Cursor<Vec<u8>>>,
    begin: u64,
    end: Option<u64>,
) -> Result<(WavSpec, Vec<i16>), EngineError> {
    let spec = reader.spec();
    if spec.channels == 0 || spec.channels > 8 || !(1..=192_000).contains(&spec.sample_rate) {
        return Err(unsupported(
            "WAVE sample rate or channel count is unsupported",
        ));
    }
    let start = frame_at(begin, spec.sample_rate)?;
    let frames = reader.duration() as usize;
    let stop = end
        .map(|time| frame_at(time, spec.sample_rate))
        .transpose()?
        .unwrap_or(frames);
    if start >= stop || stop > frames {
        return Err(invalid(
            "media-overlay audio clip lies outside its WAVE resource",
        ));
    }
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
    Ok((spec, samples))
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
        Ok(Self {
            bytes,
            first,
            byte_offset: first,
            decoder: mp3dec::Decoder::new(),
            pcm: [0; mp3dec::MAX_SAMPLES_PER_FRAME],
            sample_offset: 0,
            frame_len: 0,
            format: None,
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

    fn samples(
        &mut self,
        begin: u64,
        end: Option<u64>,
    ) -> Result<(WavSpec, Vec<i16>), EngineError> {
        if self.format.is_none() {
            self.next_frame()?;
        }
        let format = self
            .format
            .ok_or_else(|| invalid("empty MPEG audio stream"))?;
        let start = frame_at(begin, format.sample_rate)?;
        let stop = end
            .map(|time| frame_at(time, format.sample_rate))
            .transpose()?;
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
                return Ok((format, samples));
            }
            if !self.next_frame()? {
                if stop.is_some() || samples.is_empty() {
                    return Err(invalid(
                        "media-overlay audio clip lies outside its MPEG resource",
                    ));
                }
                return Ok((format, samples));
            }
        }
        Err(limited("MPEG clip exceeds its frame decoding budget"))
    }
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
