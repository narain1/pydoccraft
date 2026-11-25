//! Index bounded Vorbis packet durations and decode the requested presentation samples.
use std::ops::Range;

use hound::{SampleFormat, WavSpec};
use oxideav_core::{bits::BitReaderLsb, Demuxer};
use oxideav_vorbis::{
    parse_codebook, parse_comment_header, parse_identification_header, parse_setup_header,
    read_packet_header, setup::FloorKind, AudioDecoderState, StreamingDecoder, StreamingFrame,
    VorbisCodebook, VorbisSetupHeader, VqLookup,
};

use super::{invalid, limited, unsupported, EngineError, MAX_CLIP_SAMPLES, MAX_FRAMES_PER_CLIP};
use crate::epub::{
    charge,
    ogg::{self, Codec, Page},
    webm,
};

const MAX_PRIVATE_BYTES: usize = 128 * 1024;
const MAX_CODEBOOK_ENTRIES: usize = 256 * 1024;
const MAX_CODEBOOK_DIMENSIONS: u32 = 64;
const MAX_CODEBOOK_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CHAIN_CODEBOOK_WORK: usize = 128 * 1024 * 1024;
const MAX_LINKS: usize = 256;

pub(in crate::epub) struct Track {
    spec: WavSpec,
    frames: usize,
    links: Vec<Link>,
    playback: Option<Playback>,
}

struct Link {
    format: Format,
    index: Index,
    skip: usize,
    leading: usize,
    playable: usize,
    audible: Range<usize>,
}

struct Format {
    spec: WavSpec,
    block_sizes: [usize; 2],
    setup: Vec<u8>,
}

#[derive(Default)]
struct Index {
    bytes: Vec<u8>,
    packets: Vec<Packet>,
    previous_size: usize,
    frames: usize,
}

struct Packet {
    bytes: Range<usize>,
    end: usize,
}

struct Playback {
    link: usize,
    setup: VorbisSetupHeader,
    state: AudioDecoderState,
    decoder: StreamingDecoder,
    current: Option<usize>,
    pcm: Vec<Vec<f32>>,
}

impl Track {
    pub(super) fn open(mut reader: webm::Reader, index: u32) -> Result<Self, EngineError> {
        let stream = &reader.demux.streams()[index as usize];
        let (format, setup) = Format::parse(headers(&stream.params.extradata)?)?;
        if stream.params.channels != Some(format.spec.channels)
            || stream.params.sample_rate != Some(format.spec.sample_rate)
        {
            return Err(invalid(
                "WebM Vorbis track parameters disagree with its header",
            ));
        }
        let delay = reader
            .demux
            .track_codec_timing(index)
            .map_or(0, |t| t.codec_delay());
        if delay > webm::MAX_TIME_NS {
            return Err(limited("WebM Vorbis codec delay exceeds its timing budget"));
        }
        codebook_cache_cost(&setup)?;

        let rate = format.spec.sample_rate;
        let time_of = |frames: usize| frames as i128 * 1_000_000_000 / i128::from(rate);
        let mut packets = Index::default();
        let mut origin = None;
        let mut first_time = None;
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
                block_start = packets.frames;
                let time = unit.time()? - i128::from(delay);
                if packets.packets.is_empty() {
                    first_time = Some(time);
                } else {
                    // The priming packet has no PCM. An unlaced first audible packet
                    // has its own timestamp. Otherwise infer its position from the next block.
                    let base = *origin.get_or_insert(time - time_of(packets.frames));
                    let tick = webm::timestamp(unit.packet.time_base, 1)?;
                    if (time - base - time_of(packets.frames)).abs() > tick + 1 {
                        return Err(unsupported("discontinuous WebM Vorbis timestamps"));
                    }
                }
            }
            packets.push(&unit.packet.data, &setup, format.block_sizes)?;
            if unit.index_in_block + 1 == unit.block_count {
                end_padding = sample_frames(i128::from(unit.discard_padding), rate)?;
                if end_padding > packets.frames - block_start {
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
        let leading = sample_frames(origin.max(0), rate)?;
        let skip = sample_frames((-origin).max(0), rate)?;
        let end = packets.frames - end_padding;
        let link = Link::new(format, packets, leading, skip, end)?;
        let mut track = Self::new(vec![link])?;
        track.playback = Some(Playback::with_setup(0, &track.links[0].format, setup)?);
        Ok(track)
    }

    pub(in crate::epub) fn open_ogg(bytes: &[u8]) -> Result<Self, EngineError> {
        let pages = ogg::groups(bytes, Codec::Vorbis, None)?;
        if pages.len() > MAX_LINKS {
            return Err(limited("Ogg Vorbis exceeds its chained stream count limit"));
        }
        let mut work = 0;
        let links = pages
            .into_iter()
            .map(|segment| Link::ogg(segment.pages, &mut work))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(links)
    }

    fn new(links: Vec<Link>) -> Result<Self, EngineError> {
        let spec = links
            .first()
            .ok_or_else(|| invalid("empty Vorbis presentation"))?
            .format
            .spec;
        if links.iter().any(|link| link.format.spec != spec) {
            return Err(unsupported(
                "chained Ogg Vorbis changes its sample rate or channel count",
            ));
        }
        let mut track = Self {
            spec,
            frames: 0,
            links,
            playback: None,
        };
        track.place_links(None)?;
        Ok(track)
    }

    pub(in crate::epub) fn spec(&self) -> WavSpec {
        self.spec
    }

    pub(super) fn frames(&self) -> usize {
        self.frames
    }

    pub(in crate::epub) fn link_lengths(&self) -> impl ExactSizeIterator<Item = usize> + '_ {
        self.links.iter().map(Link::length)
    }

    pub(in crate::epub) fn align_video_links(
        &mut self,
        video: &[usize],
    ) -> Result<(), EngineError> {
        self.place_links(Some(video))
    }

    fn place_links(&mut self, video: Option<&[usize]>) -> Result<(), EngineError> {
        if video.is_some_and(|video| video.len() != self.links.len()) {
            return Err(invalid("Ogg video and Vorbis segments disagree"));
        }
        let last = self.links.len() - 1;
        let mut cursor = 0usize;
        for (index, link) in self.links.iter_mut().enumerate() {
            let end = cursor
                .checked_add(link.length())
                .ok_or_else(|| limited("Vorbis presentation exceeds its frame budget"))?;
            link.audible = cursor + link.leading..end;
            cursor = if let Some(video) = video.filter(|_| index < last) {
                cursor
                    .checked_add(link.length().max(video[index]))
                    .ok_or_else(|| limited("Ogg chain duration exceeds its frame budget"))?
            } else {
                end
            };
        }
        self.frames = cursor;
        self.playback = None;
        Ok(())
    }

    pub(super) fn samples(&mut self, start: usize, stop: usize) -> Result<Vec<i16>, EngineError> {
        if start >= stop || stop > self.frames {
            return Err(invalid("Vorbis clip lies outside its presentation"));
        }
        let channels = usize::from(self.spec.channels);
        if stop - start > MAX_CLIP_SAMPLES / channels {
            return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
        }

        let mut samples = vec![0; (stop - start) * channels];
        let first = self.links.partition_point(|link| link.audible.end <= start);
        let mut work = 0;
        for (index, link) in self.links.iter().enumerate().skip(first) {
            if link.audible.start >= stop {
                break;
            }
            let from = link.audible.start.max(start);
            let until = link.audible.end.min(stop);
            let state = match &mut self.playback {
                Some(state) if state.link == index => state,
                slot => {
                    slot.take();
                    slot.insert(Playback::new(index, &link.format)?)
                }
            };
            let raw = from - link.audible.start + link.skip..until - link.audible.start + link.skip;
            let output = &mut samples[(from - start) * channels..(until - start) * channels];
            state.read(&link.index, raw, channels, &mut work, output)?;
        }
        Ok(samples)
    }
}

impl Format {
    fn parse(
        [ident, comment, setup]: [&[u8]; 3],
    ) -> Result<(Self, VorbisSetupHeader), EngineError> {
        let ident = parse_identification_header(ident).map_err(invalid)?;
        if !(1..=2).contains(&ident.audio_channels)
            || !(1..=192_000).contains(&ident.audio_sample_rate)
        {
            return Err(unsupported(
                "Vorbis playback requires mono or stereo audio up to 192 kHz",
            ));
        }
        let spec = WavSpec {
            channels: u16::from(ident.audio_channels),
            sample_rate: ident.audio_sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        parse_comment_header(comment).map_err(invalid)?;
        bound_codebooks(setup)?;
        let parsed = parse_setup_header(setup, ident.audio_channels).map_err(invalid)?;
        let format = Self {
            spec,
            block_sizes: [
                usize::from(ident.blocksize_0),
                usize::from(ident.blocksize_1),
            ],
            setup: setup.to_vec(),
        };
        Ok((format, parsed))
    }
}

impl Link {
    fn new(
        format: Format,
        index: Index,
        leading: usize,
        skip: usize,
        end: usize,
    ) -> Result<Self, EngineError> {
        let playable = end
            .checked_sub(skip)
            .filter(|&frames| frames != 0)
            .ok_or_else(|| invalid("Vorbis stream has no playable samples"))?;
        leading
            .checked_add(playable)
            .ok_or_else(|| limited("Vorbis link exceeds its presentation frame budget"))?;
        Ok(Self {
            format,
            index,
            skip,
            leading,
            playable,
            audible: 0..0,
        })
    }

    fn length(&self) -> usize {
        self.leading + self.playable
    }

    fn ogg(pages: Vec<Page<'_>>, work: &mut usize) -> Result<Self, EngineError> {
        let empty_eos = pages.last().and_then(Page::empty_eos_granule);
        let mut packets = ogg::packets(pages);
        let mut header = || {
            let packet = packets
                .next()
                .transpose()?
                .ok_or_else(|| invalid("missing Ogg Vorbis headers"))?;
            if packet.last_in_stream() || (packet.last_in_page() && packet.absgp_page() != 0) {
                return Err(invalid("invalid Ogg Vorbis header page"));
            }
            Ok(packet)
        };
        let ident = header()?;
        let comment = header()?;
        let setup = header()?;
        if !ident.first_in_stream() || !ident.last_in_page() || !setup.last_in_page() {
            return Err(invalid("invalid Ogg Vorbis header placement"));
        }
        let (format, setup) = Format::parse([&ident.data, &comment.data, &setup.data])?;
        charge(
            work,
            codebook_cache_cost(&setup)?,
            MAX_CHAIN_CODEBOOK_WORK,
            "Ogg Vorbis exceeds its chained codebook work budget",
        )?;
        decode_state(&setup)?;
        let mut index = Index::default();
        let mut origin = None;
        let mut previous = 0;
        let mut eos = None;
        for packet in packets {
            let packet = packet?;
            index.push(&packet.data, &setup, format.block_sizes)?;
            if !packet.last_in_page() {
                continue;
            }
            let granule = i128::from(packet.absgp_page());
            if granule > i128::from(i64::MAX) || granule < previous {
                return Err(invalid("invalid Ogg Vorbis granule position"));
            }
            let position = index.frames as i128;
            if packet.last_in_stream() {
                // With all audio on one page, a shortened final granule is tail trim.
                let offset = origin.unwrap_or_else(|| (granule - position).max(0));
                if granule > position + offset {
                    return Err(invalid("invalid Ogg Vorbis final granule"));
                }
                eos = Some((offset, granule));
                break;
            }
            let offset = *origin.get_or_insert(granule - position);
            let first_overlap = index.packets.get(1).map_or(0, |packet| packet.end) as i128;
            if -offset > first_overlap || offset + position != granule {
                return Err(invalid(
                    "Ogg Vorbis granules disagree with packet durations",
                ));
            }
            previous = granule;
        }
        if let (Some(granule), Some(offset)) = (empty_eos, origin) {
            if granule != u64::MAX && i128::from(granule) != previous {
                return Err(invalid("invalid Ogg Vorbis empty end-of-stream page"));
            }
            eos = Some((offset, previous));
        }
        let (origin, granule) =
            eos.ok_or_else(|| invalid("missing Ogg Vorbis end-of-stream page"))?;
        if granule * 1_000_000_000
            > i128::from(webm::MAX_TIME_NS) * i128::from(format.spec.sample_rate)
        {
            return Err(limited("Ogg Vorbis granule exceeds its timing budget"));
        }
        let frames = |count| {
            usize::try_from(count)
                .map_err(|_| limited("Ogg Vorbis granule exceeds its frame budget"))
        };
        let leading = frames(origin.max(0))?;
        let skip = frames((-origin).max(0))?;
        let end = frames(granule - origin)?;
        Self::new(format, index, leading, skip, end)
    }
}

impl Index {
    fn push(
        &mut self,
        bytes: &[u8],
        setup: &VorbisSetupHeader,
        [short, long]: [usize; 2],
    ) -> Result<(), EngineError> {
        let header = read_packet_header(&mut BitReaderLsb::new(bytes), setup, short, long)
            .map_err(|error| invalid(format!("invalid Vorbis packet header: {error:?}")))?;
        if self.previous_size != 0 {
            self.frames += (self.previous_size + header.n) / 4;
        }
        self.previous_size = header.n;
        let offset = self.bytes.len();
        self.bytes.extend_from_slice(bytes);
        self.packets.push(Packet {
            bytes: offset..self.bytes.len(),
            end: self.frames,
        });
        Ok(())
    }
}

impl Playback {
    fn new(link: usize, format: &Format) -> Result<Self, EngineError> {
        let setup =
            parse_setup_header(&format.setup, format.spec.channels as u8).map_err(invalid)?;
        Self::with_setup(link, format, setup)
    }

    fn with_setup(
        link: usize,
        format: &Format,
        setup: VorbisSetupHeader,
    ) -> Result<Self, EngineError> {
        let state = decode_state(&setup)?;
        let [short, long] = format.block_sizes;
        Ok(Self {
            link,
            setup,
            state,
            decoder: StreamingDecoder::new(format.spec.channels as u8, short, long, 1.0),
            current: None,
            pcm: Vec::new(),
        })
    }

    fn read(
        &mut self,
        index: &Index,
        frames: Range<usize>,
        channels: usize,
        work: &mut usize,
        output: &mut [i16],
    ) -> Result<(), EngineError> {
        let mut begin = frames.start;
        let first = index.packets.partition_point(|packet| packet.end <= begin);
        let mut output = output.chunks_exact_mut(channels);
        for number in first..index.packets.len() {
            charge(
                work,
                1,
                MAX_FRAMES_PER_CLIP,
                "Vorbis clip exceeds its packet work budget",
            )?;
            self.decode(index, number)?;
            let packet_end = index.packets[number].end;
            let packet_start = index.packets[number - 1].end;
            let stop = frames.end.min(packet_end);
            for (frame, row) in (begin - packet_start..stop - packet_start).zip(&mut output) {
                for (value, channel) in row.iter_mut().zip(&self.pcm) {
                    let sample = channel[frame];
                    if !sample.is_finite() {
                        return Err(invalid("non-finite Vorbis sample"));
                    }
                    *value = (sample.clamp(-1.0, 1.0) * 32768.0).round() as i16;
                }
            }
            if stop == frames.end {
                return Ok(());
            }
            begin = stop;
        }
        Err(invalid("truncated Vorbis sample index"))
    }

    fn decode(&mut self, index: &Index, number: usize) -> Result<(), EngineError> {
        if self.current == Some(number) {
            return Ok(());
        }
        if self.current.take() != Some(number - 1) {
            // Reconstruct the preceding overlap window for a non-contiguous seek.
            self.decoder.reset();
            if !matches!(
                self.decode_packet(index, number - 1)?,
                StreamingFrame::Primed { .. }
            ) {
                return Err(invalid("Vorbis decoder did not prime its overlap window"));
            }
        }
        let StreamingFrame::Pcm {
            per_channel_pcm, ..
        } = self.decode_packet(index, number)?
        else {
            return Err(invalid("Vorbis decoder did not yield PCM"));
        };
        let expected = index.packets[number].end - index.packets[number - 1].end;
        if per_channel_pcm.len() != usize::from(self.decoder.audio_channels())
            || per_channel_pcm
                .iter()
                .any(|channel| channel.len() != expected)
        {
            return Err(invalid("Vorbis decoder changed the packet geometry"));
        }
        self.pcm = per_channel_pcm;
        self.current = Some(number);
        Ok(())
    }

    fn decode_packet(
        &mut self,
        index: &Index,
        number: usize,
    ) -> Result<StreamingFrame, EngineError> {
        let bytes = &index.bytes[index.packets[number].bytes.clone()];
        self.decoder
            .push_packet(&mut BitReaderLsb::new(bytes), &self.setup, &self.state)
            .map_err(|error| invalid(format!("malformed Vorbis audio: {error:?}")))
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
        .ok_or_else(|| invalid("missing Vorbis setup header"))?;
    let mut reader = BitReaderLsb::new(body);
    let count = reader.read_u32(8).map_err(invalid)? + 1;
    let mut entries = 0;
    for _ in 0..count {
        let mut probe = reader;
        if probe.read_u32(24).map_err(invalid)? != VorbisCodebook::SYNC_PATTERN {
            return Err(invalid("invalid Vorbis codebook signature"));
        }
        let dimensions = probe.read_u32(16).map_err(invalid)?;
        if !(1..=MAX_CODEBOOK_DIMENSIONS).contains(&dimensions) {
            return Err(limited("Vorbis codebook dimensions exceed limits"));
        }
        charge(
            &mut entries,
            probe.read_u32(24).map_err(invalid)? as usize,
            MAX_CODEBOOK_ENTRIES,
            "Vorbis codebook entries exceed limits",
        )?;
        // This native parser advances over the codeword lengths and lookup data.
        parse_codebook(&mut reader).map_err(invalid)?;
    }
    Ok(())
}

fn decode_state(setup: &VorbisSetupHeader) -> Result<AudioDecoderState, EngineError> {
    AudioDecoderState::new(setup)
        .map_err(|error| invalid(format!("invalid Vorbis setup: {error:?}")))
}

fn codebook_cache_cost(setup: &VorbisSetupHeader) -> Result<usize, EngineError> {
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
            .ok_or_else(|| invalid("invalid Vorbis codebook reference"))?;
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
            "Vorbis codebook cache exceeds 32 MiB",
        )?;
    }
    Ok(bytes)
}
