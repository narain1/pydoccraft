//! Index bounded Vorbis packet durations and decode the requested presentation samples.
use std::ops::Range;

use hound::{SampleFormat, WavSpec};
use oxideav_core::{bits::BitReaderLsb, Demuxer};
use oxideav_vorbis::{
    parse_codebook, parse_comment_header, parse_identification_header, parse_setup_header,
    read_packet_header, setup::FloorKind, AudioDecoderState, StreamingDecoder, StreamingFrame,
    VorbisCodebook, VorbisSetupHeader, VqLookup,
};

use super::{
    invalid, limited, unsupported, EngineError, MAX_CLIP_SAMPLES, MAX_FRAMES_PER_CLIP,
    VORBIS_SPEAKERS,
};
use crate::epub::{
    charge,
    ogg::{self, Codec, Page, SelectedTracks},
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
    wave_order: Vec<usize>,
    speaker_mask: Option<u32>,
    frames: usize,
    links: Vec<Link>,
    playback: Option<Playback>,
}

struct Link {
    format: Format,
    index: Index,
    spans: Vec<Span>,
    offset: usize,
}

struct Span {
    timeline: Range<usize>,
    raw_start: usize,
}

struct BlockTime {
    nanos: i128,
    first_sample: usize,
    first_packet_end: usize,
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
        let tick = webm::timestamp(stream.time_base, 1)?;
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
        let mut packets = Index::default();
        let mut blocks = Vec::<BlockTime>::new();
        let mut end_padding = 0;
        while let Some(unit) = reader.next()? {
            if unit.packet.stream_index != index {
                continue;
            }
            if end_padding != 0 || unit.discard_padding < 0 {
                return Err(unsupported(
                    "WebM Vorbis discard padding outside the final block",
                ));
            }
            let first_sample = packets.frames;
            packets.push(&unit.packet.data, &setup, format.block_sizes)?;
            if unit.index_in_block == 0 {
                let nanos = unit.time()? - i128::from(delay);
                if blocks.last().is_some_and(|block| nanos < block.nanos) {
                    return Err(unsupported("backward WebM Vorbis timestamps"));
                }
                blocks.push(BlockTime {
                    nanos,
                    first_sample,
                    first_packet_end: packets.frames,
                });
            }
            if unit.index_in_block + 1 == unit.block_count {
                end_padding = sample_frames(i128::from(unit.discard_padding), rate)?;
                let block_start = blocks.last().expect("indexed WebM block").first_sample;
                if end_padding > packets.frames - block_start {
                    return Err(invalid(
                        "WebM Vorbis discard padding exceeds its final block",
                    ));
                }
            }
        }
        let spans = webm_spans(&blocks, tick, rate, packets.frames - end_padding)?;
        let link = Link::new(format, packets, spans)?;
        let mut track = Self::new(vec![link])?;
        track.playback = Some(Playback::with_setup(0, &track.links[0].format, setup)?);
        Ok(track)
    }

    pub(in crate::epub) fn open_ogg(
        bytes: &[u8],
        named: Option<&SelectedTracks>,
    ) -> Result<Self, EngineError> {
        let pages = ogg::groups(bytes, Codec::Vorbis, None, named)?;
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

    pub(super) fn open_ogg_link(
        pages: Vec<Page<'_>>,
        work: &mut usize,
    ) -> Result<Self, EngineError> {
        Self::new(vec![Link::ogg(pages, work)?])
    }

    pub(super) fn release_playback(&mut self) {
        self.playback = None;
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
        let speakers = VORBIS_SPEAKERS[usize::from(spec.channels - 1)];
        let speaker_mask =
            (spec.channels > 2).then(|| speakers.iter().fold(0, |mask, flag| mask | flag));
        let mut wave_order: Vec<_> = (0..speakers.len()).collect();
        wave_order.sort_unstable_by_key(|&channel| speakers[channel]);

        let mut track = Self {
            spec,
            wave_order,
            speaker_mask,
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

    pub(super) fn speaker_mask(&self) -> Option<u32> {
        self.speaker_mask
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
            link.offset = cursor;
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
        self.samples_with_work(start, stop, &mut 0)
    }

    pub(super) fn samples_with_work(
        &mut self,
        start: usize,
        stop: usize,
        work: &mut usize,
    ) -> Result<Vec<i16>, EngineError> {
        if start >= stop || stop > self.frames {
            return Err(invalid("Vorbis clip lies outside its presentation"));
        }
        let channels = usize::from(self.spec.channels);
        if stop - start > MAX_CLIP_SAMPLES / channels {
            return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
        }

        let mut samples = vec![0; (stop - start) * channels];
        let first = self
            .links
            .partition_point(|link| link.offset + link.length() <= start);
        for (index, link) in self.links.iter().enumerate().skip(first) {
            if link.offset >= stop {
                break;
            }
            let local = start.saturating_sub(link.offset)..stop - link.offset;
            let first = link
                .spans
                .partition_point(|span| span.timeline.end <= local.start);
            for span in &link.spans[first..] {
                if span.timeline.start >= local.end {
                    break;
                }
                let from = span.timeline.start.max(local.start);
                let until = span.timeline.end.min(local.end);
                let state = match &mut self.playback {
                    Some(state) if state.link == index => state,
                    slot => {
                        slot.take();
                        slot.insert(Playback::new(index, &link.format)?)
                    }
                };
                let raw_start = span.raw_start + (from - span.timeline.start);
                let count = until - from;
                let raw = raw_start..raw_start + count;
                let offset = (link.offset + from - start) * channels;
                let output = &mut samples[offset..offset + count * channels];
                state.read(&link.index, raw, &self.wave_order, work, output)?;
            }
        }
        Ok(samples)
    }
}

impl Format {
    fn parse(
        [ident, comment, setup]: [&[u8]; 3],
    ) -> Result<(Self, VorbisSetupHeader), EngineError> {
        let ident = parse_identification_header(ident).map_err(invalid)?;
        if usize::from(ident.audio_channels) > VORBIS_SPEAKERS.len() {
            return Err(unsupported(
                "Vorbis channel layout has no defined speaker order",
            ));
        }
        if !(1..=192_000).contains(&ident.audio_sample_rate) {
            return Err(unsupported(
                "Vorbis playback requires a sample rate up to 192 kHz",
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
    fn new(format: Format, index: Index, spans: Vec<Span>) -> Result<Self, EngineError> {
        if spans.is_empty() {
            return Err(invalid("Vorbis stream has no playable samples"));
        }
        Ok(Self {
            format,
            index,
            spans,
            offset: 0,
        })
    }

    fn length(&self) -> usize {
        self.spans
            .last()
            .expect("nonempty Vorbis spans")
            .timeline
            .end
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
        let end = frames(granule)?;
        let spans = if leading < end {
            vec![Span {
                timeline: leading..end,
                raw_start: skip,
            }]
        } else {
            Vec::new()
        };
        Self::new(format, index, spans)
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
        wave_order: &[usize],
        work: &mut usize,
        output: &mut [i16],
    ) -> Result<(), EngineError> {
        let mut begin = frames.start;
        let first = index.packets.partition_point(|packet| packet.end <= begin);
        let mut output = output.chunks_exact_mut(wave_order.len());
        for number in first..index.packets.len() {
            charge(
                work,
                wave_order.len().max(2),
                MAX_FRAMES_PER_CLIP * 2,
                "Vorbis clip exceeds its packet work budget",
            )?;
            self.decode(index, number)?;
            let packet_end = index.packets[number].end;
            let packet_start = index.packets[number - 1].end;
            let stop = frames.end.min(packet_end);
            for (frame, row) in (begin - packet_start..stop - packet_start).zip(&mut output) {
                for (value, &channel) in row.iter_mut().zip(wave_order) {
                    let sample = self.pcm[channel][frame];
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

fn webm_spans(
    blocks: &[BlockTime],
    tick: i128,
    rate: u32,
    frames: usize,
) -> Result<Vec<Span>, EngineError> {
    let (priming, audible) = blocks
        .split_first()
        .ok_or_else(|| invalid("WebM Vorbis has no packets"))?;
    let time_of = |frame: usize| frame as i128 * 1_000_000_000 / i128::from(rate);
    let start_offset = |block: &BlockTime| block.nanos - time_of(block.first_sample);
    // The priming packet produces no PCM. For laced audio, infer its output
    // origin from the first later Block when available.
    let origin = audible.first().map_or(priming.nanos, start_offset);
    let matches = |offset: i128, observed: i128| (observed - offset).abs() <= tick + 1;
    let start_timed = audible
        .iter()
        .all(|block| matches(origin, start_offset(block)));
    // Legacy muxers used the end of the Vorbis overlap output as the packet
    // timestamp. Accept that convention only when it fits the complete track.
    let end_timed = !start_timed
        && blocks
            .iter()
            .all(|block| matches(priming.nanos, block.nanos - time_of(block.first_packet_end)));

    let mut spans = Vec::new();
    if start_timed || end_timed {
        let offset = if end_timed { priming.nanos } else { origin };
        append_span(&mut spans, 0..frames, offset, rate)?;
    } else {
        let mut first = 0;
        let mut offset = origin;
        for block in audible
            .iter()
            .take_while(|block| block.first_sample < frames)
        {
            let observed = start_offset(block);
            if !matches(offset, observed) {
                append_span(&mut spans, first..block.first_sample, offset, rate)?;
                first = block.first_sample;
                offset = observed;
            }
        }
        append_span(&mut spans, first..frames, offset, rate)?;
    }
    Ok(spans)
}

fn append_span(
    spans: &mut Vec<Span>,
    raw: Range<usize>,
    offset_ns: i128,
    rate: u32,
) -> Result<(), EngineError> {
    let offset = sample_frames(offset_ns.abs(), rate)? as i128 * offset_ns.signum();
    let start = raw.start as i128 + offset;
    let end = raw.end as i128 + offset;
    // Output begins at time zero. On overlap, keep the preceding samples and
    // skip the obstructed beginning of the later span.
    let first = start.max(spans.last().map_or(0, |span| span.timeline.end) as i128);
    if first < end {
        let frame = |position| {
            usize::try_from(position)
                .map_err(|_| limited("WebM Vorbis presentation exceeds its frame budget"))
        };
        spans.push(Span {
            timeline: frame(first)?..frame(end)?,
            raw_start: raw.start + frame(first - start)?,
        });
    }
    Ok(())
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
