//! Index and decode one bounded Vorbis soundtrack from WebM.
use std::ops::Range;

use hound::{SampleFormat, WavSpec};
use oxideav_core::{bits::BitReaderLsb, Demuxer};
use oxideav_vorbis::{
    parse_codebook, parse_comment_header, parse_identification_header, parse_setup_header,
    read_packet_header, setup::FloorKind, AudioDecoderState, StreamingDecoder, StreamingFrame,
    VorbisCodebook, VorbisSetupHeader, VqLookup,
};

use super::{invalid, limited, unsupported, EngineError, MAX_CLIP_SAMPLES, MAX_FRAMES_PER_CLIP};
use crate::epub::{charge, webm};

const MAX_PRIVATE_BYTES: usize = 128 * 1024;
const MAX_CODEBOOK_ENTRIES: usize = 256 * 1024;
const MAX_CODEBOOK_DIMENSIONS: u32 = 64;
const MAX_CODEBOOK_CACHE_BYTES: usize = 32 * 1024 * 1024;

pub(super) struct Track {
    spec: WavSpec,
    setup: VorbisSetupHeader,
    state: AudioDecoderState,
    decoder: StreamingDecoder,
    bytes: Vec<u8>,
    packets: Vec<Packet>,
    timeline: Range<usize>,
    skip: usize,
    current: Option<usize>,
    pcm: Vec<Vec<f32>>,
}

struct Packet {
    bytes: Range<usize>,
    end: usize,
}

impl Track {
    pub(super) fn open(mut reader: webm::Reader, index: u32) -> Result<Self, EngineError> {
        let stream = &reader.demux.streams()[index as usize];
        let [ident, comment, setup] = headers(&stream.params.extradata)?;
        let ident = parse_identification_header(ident).map_err(invalid)?;
        if !(1..=2).contains(&ident.audio_channels)
            || !(1..=192_000).contains(&ident.audio_sample_rate)
        {
            return Err(unsupported(
                "WebM Vorbis requires mono or stereo audio up to 192 kHz",
            ));
        }
        let spec = WavSpec {
            channels: u16::from(ident.audio_channels),
            sample_rate: ident.audio_sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        if stream.params.channels != Some(spec.channels)
            || stream.params.sample_rate != Some(spec.sample_rate)
        {
            return Err(invalid(
                "WebM Vorbis track parameters disagree with its header",
            ));
        }
        parse_comment_header(comment).map_err(invalid)?;
        bound_codebooks(setup)?;
        let setup = parse_setup_header(setup, ident.audio_channels).map_err(invalid)?;
        bound_cache(&setup)?;
        let state = AudioDecoderState::new(&setup)
            .map_err(|error| invalid(format!("invalid WebM Vorbis setup: {error:?}")))?;

        let delay = reader
            .demux
            .track_codec_timing(index)
            .map_or(0, |t| t.codec_delay());
        if delay > webm::MAX_TIME_NS {
            return Err(limited("WebM Vorbis codec delay exceeds its timing budget"));
        }
        let time_of = |frames: usize| frames as i128 * 1_000_000_000 / i128::from(spec.sample_rate);
        let blocksize_0 = usize::from(ident.blocksize_0);
        let blocksize_1 = usize::from(ident.blocksize_1);
        let mut bytes = Vec::new();
        let mut packets = Vec::new();
        let mut frames = 0usize;
        let mut origin = None;
        let mut first_time = None;
        let mut previous_block_size = None;
        let mut end_padding = 0;
        let mut block_start = 0;
        while let Some(unit) = reader.next()? {
            if unit.packet.stream_index != index {
                continue;
            }
            if end_padding != 0 || unit.discard_padding < 0 {
                return Err(unsupported(
                    "WebM Vorbis discard padding outside the final block",
                ));
            }
            if unit.index_in_block == 0 {
                block_start = frames;
                let time = unit.time()? - i128::from(delay);
                if previous_block_size.is_none() {
                    first_time = Some(time);
                } else {
                    // The first Vorbis packet primes the overlap window and has no PCM.
                    // An unlaced first audible packet has its own presentation timestamp.
                    let base = *origin.get_or_insert(time - time_of(frames));
                    let tick = webm::timestamp(unit.packet.time_base, 1)?;
                    if (time - base - time_of(frames)).abs() > tick + 1 {
                        return Err(unsupported("discontinuous WebM Vorbis timestamps"));
                    }
                }
            }
            let packet = &unit.packet.data;
            let header = read_packet_header(
                &mut BitReaderLsb::new(packet),
                &setup,
                blocksize_0,
                blocksize_1,
            )
            .map_err(|error| invalid(format!("invalid WebM Vorbis packet header: {error:?}")))?;
            if let Some(previous) = previous_block_size.replace(header.n) {
                frames += (previous + header.n) / 4;
            }
            let offset = bytes.len();
            bytes.extend_from_slice(packet);
            packets.push(Packet {
                bytes: offset..bytes.len(),
                end: frames,
            });
            if unit.index_in_block + 1 == unit.block_count {
                end_padding = sample_frames(i128::from(unit.discard_padding), spec.sample_rate)?;
                if end_padding > frames - block_start {
                    return Err(invalid(
                        "WebM Vorbis discard padding exceeds its final block",
                    ));
                }
            }
        }
        // A single lace can contain the priming packet and all audible packets.
        let origin = origin
            .or(first_time)
            .ok_or_else(|| invalid("WebM Vorbis has no packets"))?;
        let start = sample_frames(origin.max(0), spec.sample_rate)?;
        let skip = sample_frames((-origin).max(0), spec.sample_rate)?;
        let playable = frames
            .checked_sub(skip)
            .and_then(|frames| frames.checked_sub(end_padding))
            .filter(|&count| count != 0)
            .ok_or_else(|| invalid("WebM Vorbis has no playable samples"))?;
        let end = start
            .checked_add(playable)
            .ok_or_else(|| limited("WebM Vorbis presentation exceeds its frame budget"))?;
        Ok(Self {
            spec,
            setup,
            state,
            bytes,
            packets,
            skip,
            timeline: start..end,
            decoder: StreamingDecoder::new(ident.audio_channels, blocksize_0, blocksize_1, 1.0),
            current: None,
            pcm: Vec::new(),
        })
    }

    pub(super) fn spec(&self) -> WavSpec {
        self.spec
    }

    pub(super) fn frames(&self) -> usize {
        self.timeline.end
    }

    pub(super) fn samples(&mut self, start: usize, stop: usize) -> Result<Vec<i16>, EngineError> {
        if start >= stop || stop > self.frames() {
            return Err(invalid("Vorbis clip lies outside its presentation"));
        }
        let channels = usize::from(self.spec.channels);
        if stop - start > MAX_CLIP_SAMPLES / channels {
            return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
        }
        let mut samples = Vec::with_capacity((stop - start) * channels);
        let leading = self.timeline.start.min(stop).saturating_sub(start);
        samples.resize(leading * channels, 0);
        if leading == stop - start {
            return Ok(samples);
        }
        let mut begin = start + leading - self.timeline.start + self.skip;
        let end = stop - self.timeline.start + self.skip;
        let first = self.packets.partition_point(|packet| packet.end <= begin);
        for index in first..self.packets.len().min(first + MAX_FRAMES_PER_CLIP) {
            self.decode(index)?;
            let packet_end = self.packets[index].end;
            let packet_start = self.packets[index - 1].end;
            let stop = end.min(packet_end);
            for frame in begin - packet_start..stop - packet_start {
                for channel in &self.pcm {
                    let value = channel[frame];
                    if !value.is_finite() {
                        return Err(invalid("non-finite WebM Vorbis sample"));
                    }
                    samples.push((value.clamp(-1.0, 1.0) * 32768.0).round() as i16);
                }
            }
            if stop == end {
                return Ok(samples);
            }
            begin = stop;
        }
        Err(limited("WebM Vorbis clip exceeds its packet work budget"))
    }

    fn decode(&mut self, index: usize) -> Result<(), EngineError> {
        if self.current == Some(index) {
            return Ok(());
        }
        if self.current.take() != Some(index - 1) {
            // Vorbis reconstructs a packet with only its preceding overlap window.
            self.decoder.reset();
            if !matches!(
                self.decode_packet(index - 1)?,
                StreamingFrame::Primed { .. }
            ) {
                return Err(invalid(
                    "WebM Vorbis decoder did not prime its overlap window",
                ));
            }
        }
        let StreamingFrame::Pcm {
            per_channel_pcm, ..
        } = self.decode_packet(index)?
        else {
            return Err(invalid("WebM Vorbis decoder did not yield PCM"));
        };
        let expected = self.packets[index].end - self.packets[index - 1].end;
        if per_channel_pcm.len() != usize::from(self.spec.channels)
            || per_channel_pcm
                .iter()
                .any(|channel| channel.len() != expected)
        {
            return Err(invalid("WebM Vorbis decoder changed the packet geometry"));
        }
        self.pcm = per_channel_pcm;
        self.current = Some(index);
        Ok(())
    }

    fn decode_packet(&mut self, index: usize) -> Result<StreamingFrame, EngineError> {
        let bytes = &self.bytes[self.packets[index].bytes.clone()];
        self.decoder
            .push_packet(&mut BitReaderLsb::new(bytes), &self.setup, &self.state)
            .map_err(|error| invalid(format!("malformed WebM Vorbis audio: {error:?}")))
    }
}

fn sample_frames(nanos: i128, rate: u32) -> Result<usize, EngineError> {
    if !(0..=i128::from(webm::MAX_TIME_NS)).contains(&nanos) {
        return Err(limited("WebM Vorbis time exceeds its timing budget"));
    }
    usize::try_from((nanos * i128::from(rate) + 500_000_000) / 1_000_000_000)
        .map_err(|_| limited("WebM Vorbis time exceeds its frame budget"))
}

fn headers(bytes: &[u8]) -> Result<[&[u8]; 3], EngineError> {
    if bytes.len() > MAX_PRIVATE_BYTES {
        return Err(limited("WebM Vorbis codec private data exceeds 128 KiB"));
    }
    let Some(mut rest) = bytes.strip_prefix(&[2]) else {
        return Err(invalid("WebM Vorbis requires three Xiph-laced headers"));
    };
    let mut lengths = [0, 0];
    for length in &mut lengths {
        loop {
            let (&byte, next) = rest
                .split_first()
                .ok_or_else(|| invalid("truncated WebM Vorbis lacing"))?;
            *length += usize::from(byte);
            rest = next;
            if byte != 255 {
                break;
            }
        }
    }
    let (ident, rest) = rest
        .split_at_checked(lengths[0])
        .ok_or_else(|| invalid("truncated WebM Vorbis identification header"))?;
    let (comment, setup) = rest
        .split_at_checked(lengths[1])
        .ok_or_else(|| invalid("truncated WebM Vorbis comment header"))?;
    Ok([ident, comment, setup])
}

fn bound_codebooks(setup: &[u8]) -> Result<(), EngineError> {
    let body = setup
        .strip_prefix(b"\x05vorbis")
        .ok_or_else(|| invalid("missing WebM Vorbis setup header"))?;
    let mut reader = BitReaderLsb::new(body);
    let count = reader.read_u32(8).map_err(invalid)? + 1;
    let mut entries = 0;
    for _ in 0..count {
        let mut probe = reader;
        if probe.read_u32(24).map_err(invalid)? != VorbisCodebook::SYNC_PATTERN {
            return Err(invalid("invalid WebM Vorbis codebook signature"));
        }
        let dimensions = probe.read_u32(16).map_err(invalid)?;
        if !(1..=MAX_CODEBOOK_DIMENSIONS).contains(&dimensions) {
            return Err(limited("WebM Vorbis codebook dimensions exceed limits"));
        }
        charge(
            &mut entries,
            probe.read_u32(24).map_err(invalid)? as usize,
            MAX_CODEBOOK_ENTRIES,
            "WebM Vorbis codebook entries exceed limits",
        )?;
        // This native parser advances over the codeword lengths and lookup data.
        parse_codebook(&mut reader).map_err(invalid)?;
    }
    Ok(())
}

fn bound_cache(setup: &VorbisSetupHeader) -> Result<(), EngineError> {
    let mut references = Vec::new();
    for floor in &setup.floors {
        match &floor.kind {
            FloorKind::Type0(floor) => references.extend(floor.book_list.iter().copied()),
            FloorKind::Type1(floor) => {
                for class in &floor.classes {
                    references.extend(class.masterbook);
                    references.extend(class.subclass_books.iter().flatten().copied());
                }
            }
        }
    }
    for residue in &setup.residues {
        references.push(residue.classbook);
        references.extend(residue.books.iter().flatten().flatten().copied());
    }
    let mut bytes = 0;
    for id in references {
        let book = setup
            .codebooks
            .get(usize::from(id))
            .ok_or_else(|| invalid("invalid WebM Vorbis codebook reference"))?;
        let lookup = match &book.lookup {
            VqLookup::None => 0,
            VqLookup::Lattice { multiplicands, .. }
            | VqLookup::Tessellation { multiplicands, .. } => {
                multiplicands.len() * size_of::<u32>()
            }
        };
        // Floor and residue decoders may each retain copies of a book and its tree.
        charge(
            &mut bytes,
            (book.entries as usize) * 64 + lookup,
            MAX_CODEBOOK_CACHE_BYTES,
            "WebM Vorbis codebook cache exceeds 32 MiB",
        )?;
    }
    Ok(())
}
