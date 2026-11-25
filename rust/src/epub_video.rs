//! Stream local EPUB video on its container's presentation timeline.
use std::{io::Cursor, ops::Range, sync::Arc};

use oxideav_core::{
    CodecId, CodecParameters, CodecRegistry, Decoder, Demuxer, Frame, MediaType, Packet,
    PixelFormat, TimeBase, VideoFrame,
};
use oxideav_h264::{
    avcc::AvcDecoderConfigurationRecord,
    h264_decoder::H264CodecDecoder,
    nal::{parse_nal_unit, AvccSplitter},
    pps::Pps,
    sps::Sps,
};
use oxideav_mp4::demux::{open_typed, EditListEntry, SampleInfo};

use super::{
    audio::MAX_ENCODED_BYTES,
    invalid, limited,
    mp4::{Mp4Bounds, MAX_ACCESS_UNITS},
    unsupported, validate_archive_path, Book,
};
use crate::EngineError;

#[path = "epub_theora.rs"]
mod theora;
#[path = "epub_webm_video.rs"]
mod webm;

pub(super) use theora::{align_audio, align_vorbis_audio};

const MAX_PIXELS: u32 = 1920 * 1088;
const MAX_PACKET: usize = 2 * 1024 * 1024;
const MAX_NALS: usize = 256;
const MAX_PACKETS_PER_FRAME: usize = 512;

pub(crate) enum VideoSource {
    Avc(Box<AvcSource>),
    Theora(Box<theora::TheoraSource>),
    Webm(Box<webm::WebmSource>),
}

pub(crate) struct AvcSource {
    bytes: Arc<[u8]>,
    samples: Vec<SampleInfo>,
    timeline: Vec<FrameTime>,
    next_packet: usize,
    next_frame: usize,
    decoder: H264CodecDecoder,
    flushed: bool,
    geometry: Geometry,
    pixel_format: PixelFormat,
    stream: u32,
    time_base: TimeBase,
    nal_length: u8,
    clip: Range<u64>,
}

struct FrameTime {
    pts: i64,
    interval: Range<u64>,
}

pub(crate) struct VideoImage {
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub aspect: (u32, u32),
    pub start_us: u64,
    pub duration_us: u64,
}

struct Geometry {
    width: u32,
    height: u32,
    aspect: (u32, u32),
    full_range: bool,
    kr: f32,
    kb: f32,
}

impl AvcSource {
    fn pixel_format(sps: &Sps) -> Result<PixelFormat, EngineError> {
        use PixelFormat::*;

        if !sps.frame_mbs_only_flag
            || sps.separate_colour_plane_flag
            || sps.bit_depth_chroma_minus8 != sps.bit_depth_luma_minus8
        {
            return Err(unsupported("unsupported AVC picture structure"));
        }
        match (sps.chroma_format_idc, sps.bit_depth_luma_minus8) {
            (1, 0) => Ok(Yuv420P),
            (2, 0) => Ok(Yuv422P),
            (3, 0) => Ok(Yuv444P),
            (1, 2) => Ok(Yuv420P10Le),
            (2, 2) => Ok(Yuv422P10Le),
            (3, 2) => Ok(Yuv444P10Le),
            _ => Err(unsupported(
                "embedded AVC requires 8-bit or 10-bit planar YUV",
            )),
        }
    }
}

impl VideoSource {
    pub(crate) fn open(
        epub: &[u8],
        path: &str,
        media: &str,
        start_us: u64,
        end_us: Option<u64>,
        video_track: Option<usize>,
    ) -> Result<Self, EngineError> {
        if video_track.is_some() && media != "video/webm" {
            return Err(unsupported(
                "WebM video track selection requires a local WebM source",
            ));
        }
        validate_archive_path(path)?;
        let bytes = Book::open(epub)?.read(path, MAX_ENCODED_BYTES)?;
        match media {
            "video/mp4" => std::panic::catch_unwind(|| AvcSource::decode(bytes, start_us, end_us))
                .map_err(|_| invalid("malformed embedded MP4 video"))?
                .map(|source| Self::Avc(Box::new(source))),
            "video/ogg" | "application/ogg" => {
                std::panic::catch_unwind(|| theora::TheoraSource::open(&bytes, start_us, end_us))
                    .map_err(|_| invalid("malformed embedded Ogg video"))?
                    .map(|source| Self::Theora(Box::new(source)))
            }
            "video/webm" => std::panic::catch_unwind(|| {
                webm::WebmSource::open(bytes, start_us, end_us, video_track)
            })
            .map_err(|_| invalid("malformed embedded WebM video"))?
            .map(|source| Self::Webm(Box::new(source))),
            _ => Err(unsupported("unsupported EPUB video media type")),
        }
    }

    pub(crate) fn next(&mut self) -> Result<Option<VideoImage>, EngineError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match self {
            Self::Avc(source) => source.next_image(),
            Self::Theora(source) => source.next_image(),
            Self::Webm(source) => source.next_image(),
        }))
        .map_err(|_| invalid("malformed embedded video frame"))?
    }
}

impl AvcSource {
    fn decode(bytes: Vec<u8>, start_us: u64, end_us: Option<u64>) -> Result<Self, EngineError> {
        let guard = Mp4Bounds::inspect(&bytes)?;
        let bytes: Arc<[u8]> = bytes.into();
        let demux = open_typed(Box::new(Cursor::new(bytes.clone())), &CodecRegistry::new())
            .map_err(invalid)?;
        let mut tracks = demux
            .streams()
            .iter()
            .filter(|stream| stream.params.media_type == MediaType::Video);
        let stream = tracks
            .next()
            .ok_or_else(|| unsupported("MP4 has no video track"))?;
        if tracks.next().is_some() {
            return Err(unsupported("embedded MP4 requires one video track"));
        }
        let entry = demux.sample_entry(stream.index, 1);
        if demux.sample_entry_count(stream.index) != 1
            || entry.is_none_or(|entry| entry.format != *b"avc1" || entry.data_reference_index != 1)
        {
            return Err(unsupported("embedded MP4 requires a local AVC video track"));
        }
        const IDENTITY: [i32; 9] = [65536, 0, 0, 0, 65536, 0, 0, 0, 1073741824];
        if demux
            .track_header(stream.index)
            .is_some_and(|track| track.matrix != IDENTITY)
        {
            return Err(unsupported("rotated or transformed embedded MP4 video"));
        }
        let extra = &stream.params.extradata;
        let (geometry, nal_length, pixel_format) = Geometry::from_avc(extra, &stream.params)?;
        let samples = demux.samples(stream.index);
        if samples.is_empty() || samples.len() > MAX_ACCESS_UNITS {
            return Err(limited("MP4 video access unit count is out of range"));
        }
        for sample in &samples {
            let range = sample_range(sample)?;
            if range.is_empty()
                || range.len() > MAX_PACKET
                || !guard.contains_media(range)
                || sample.sample_description_index != 1
                || sample.duration <= 0
            {
                return Err(invalid("invalid MP4 video access unit"));
            }
        }
        let (timeline, clip, first_packet) = playback_range(
            &samples,
            stream.time_base,
            demux.edit_list(stream.index),
            demux.movie_timescale(),
            start_us,
            end_us,
        )?;
        let mut decoder = H264CodecDecoder::new(CodecId::new("h264"));
        decoder.consume_extradata(extra).map_err(invalid)?;
        Ok(Self {
            bytes,
            samples,
            timeline,
            next_packet: first_packet,
            next_frame: 0,
            decoder,
            flushed: false,
            geometry,
            pixel_format,
            stream: stream.index,
            time_base: stream.time_base,
            nal_length,
            clip,
        })
    }

    fn next_image(&mut self) -> Result<Option<VideoImage>, EngineError> {
        if self.next_frame == self.timeline.len() {
            return Ok(None);
        }
        for _ in 0..MAX_PACKETS_PER_FRAME {
            match self.decoder.receive_frame() {
                Ok(Frame::Video(frame)) => {
                    let pts = frame
                        .pts
                        .ok_or_else(|| invalid("AVC frame has no timestamp"))?;
                    let expected = &self.timeline[self.next_frame];
                    if pts < expected.pts && self.next_frame == 0 {
                        continue;
                    }
                    if pts != expected.pts {
                        return Err(invalid(
                            "AVC frames disagree with the MP4 presentation timeline",
                        ));
                    }
                    let output = self
                        .decoder
                        .output_params()
                        .ok_or_else(|| invalid("missing AVC video geometry"))?;
                    if output.pixel_format != Some(self.pixel_format)
                        || output.width != Some(self.geometry.width)
                        || output.height != Some(self.geometry.height)
                    {
                        return Err(unsupported("AVC video changed geometry or pixel format"));
                    }
                    self.next_frame += 1;
                    let start_us = expected.interval.start.max(self.clip.start);
                    let stop = expected.interval.end.min(self.clip.end);
                    return self
                        .geometry
                        .avc_image(frame, self.pixel_format, start_us, stop - start_us)
                        .map(Some);
                }
                Ok(_) => return Err(unsupported("AVC video changed stream parameters")),
                Err(error) if error.is_starved() => self.feed()?,
                Err(error) => return Err(invalid(error)),
            }
        }
        Err(limited("MP4 video decode exceeds its per-frame work limit"))
    }

    fn feed(&mut self) -> Result<(), EngineError> {
        if let Some(sample) = self.samples.get(self.next_packet) {
            let data = &self.bytes[sample_range(sample)?];
            let mut idr = false;
            let mut picture = false;
            for (index, nal) in AvccSplitter::new(data, self.nal_length)
                .map_err(invalid)?
                .enumerate()
            {
                if index == MAX_NALS {
                    return Err(limited("too many NAL units in an AVC sample"));
                }
                let kind = nal
                    .map_err(invalid)?
                    .first()
                    .ok_or_else(|| invalid("empty AVC NAL unit"))?
                    & 31;
                match kind {
                    1 => picture = true,
                    5 => {
                        picture = true;
                        idr = true;
                    }
                    6 | 9..=12 => {}
                    _ => return Err(unsupported("unsupported NAL unit in embedded AVC video")),
                }
            }
            if !picture || (sample.sync && !idr) {
                return Err(invalid("invalid AVC picture or MP4 sync sample"));
            }
            let packet = Packet::new(self.stream, self.time_base, data.to_vec())
                .with_pts(sample.composition_time)
                .with_dts(sample.decode_time)
                .with_keyframe(sample.sync);
            self.decoder.send_packet(&packet).map_err(invalid)?;
            self.next_packet += 1;
        } else if !self.flushed {
            self.decoder.flush().map_err(invalid)?;
            self.flushed = true;
        } else {
            return Err(invalid("AVC video ended before its declared frames"));
        }
        if self.decoder.decode_error_count() != 0 {
            return Err(invalid("AVC decoder failed to reconstruct a frame"));
        }
        Ok(())
    }
}

fn sample_range(sample: &SampleInfo) -> Result<Range<usize>, EngineError> {
    let start = usize::try_from(sample.offset).map_err(invalid)?;
    let end = start
        .checked_add(sample.size as usize)
        .ok_or_else(|| invalid("invalid MP4 video byte range"))?;
    Ok(start..end)
}

fn scale_us(value: i128, num: i64, den: i64) -> Result<u64, EngineError> {
    if num <= 0 || den <= 0 {
        return Err(invalid("invalid MP4 video timescale"));
    }
    let value = value
        .checked_mul(i128::from(num) * 1_000_000)
        .ok_or_else(|| invalid("invalid MP4 video timestamp"))?
        / i128::from(den);
    u64::try_from(value).map_err(|_| invalid("invalid MP4 video timestamp"))
}

fn playback_range(
    samples: &[SampleInfo],
    base: TimeBase,
    edits: &[EditListEntry],
    movie_timescale: u32,
    start_us: u64,
    end_us: Option<u64>,
) -> Result<(Vec<FrameTime>, Range<u64>, usize), EngineError> {
    let mut display: Vec<_> = (0..samples.len()).collect();
    display.sort_unstable_by_key(|&i| samples[i].composition_time);
    let first = samples[display[0]].composition_time;
    let stop = {
        let sample = &samples[*display.last().unwrap_or(&0)];
        sample
            .composition_time
            .checked_add(sample.duration)
            .ok_or_else(|| invalid("MP4 video duration overflow"))?
    };
    let (origin, edit_duration) = match edits {
        [] => (first, None),
        [edit]
            if edit.media_time >= first
                && edit.media_rate_integer == 1
                && edit.media_rate_fraction == 0 =>
        {
            let duration = scale_us(
                i128::from(edit.segment_duration),
                1,
                i64::from(movie_timescale),
            )?;
            (edit.media_time, Some(duration))
        }
        _ => {
            return Err(unsupported(
                "MP4 video edit lists require a continuous presentation",
            ))
        }
    };
    let since_origin = |pts: i64| {
        scale_us(
            (i128::from(pts) - i128::from(origin)).max(0),
            base.0.num,
            base.0.den,
        )
    };
    let available = since_origin(stop)?;
    let duration = available.min(edit_duration.unwrap_or(available));
    let clip = start_us..end_us.unwrap_or(duration).min(duration);
    if clip.is_empty() {
        return Err(EngineError::InvalidArgument(
            "EPUB video clip lies outside its presentation".into(),
        ));
    }
    let mut timeline = Vec::new();
    let mut first_packet = samples.len();
    for (ordinal, &index) in display.iter().enumerate() {
        let pts = samples[index].composition_time;
        let next = display
            .get(ordinal + 1)
            .map_or(stop, |&i| samples[i].composition_time);
        if next <= pts {
            return Err(invalid(
                "duplicate or unordered MP4 video presentation timestamp",
            ));
        }
        let interval = since_origin(pts)?..since_origin(next)?;
        if interval.start < clip.end && interval.end > clip.start {
            if interval.is_empty() {
                return Err(invalid("MP4 video frames are shorter than a microsecond"));
            }
            first_packet = first_packet.min(index);
            timeline.push(FrameTime { pts, interval });
        }
    }
    let first_packet = samples
        .get(..=first_packet)
        .and_then(|slice| slice.iter().rposition(|sample| sample.sync))
        .ok_or_else(|| invalid("MP4 video clip has no preceding sync sample"))?;
    Ok((timeline, clip, first_packet))
}

impl Geometry {
    fn from_avc(
        extra: &[u8],
        params: &CodecParameters,
    ) -> Result<(Self, u8, PixelFormat), EngineError> {
        if extra.len() > 16 * 1024 {
            return Err(limited("AVC configuration exceeds 16 KiB"));
        }
        let config = AvcDecoderConfigurationRecord::parse(extra).map_err(invalid)?;
        if config.sps.len() != 1 || config.pps.len() != 1 || !config.sps_ext.is_empty() {
            return Err(unsupported("embedded AVC requires one fixed parameter set"));
        }
        let sps =
            Sps::parse(&parse_nal_unit(&config.sps[0]).map_err(invalid)?.rbsp).map_err(invalid)?;
        let pixel_format = AvcSource::pixel_format(&sps)?;
        let geometry_limit = || limited("AVC frame geometry or reference buffering exceeds limits");
        let coded_width = sps
            .pic_width_in_mbs_minus1
            .checked_add(1)
            .and_then(|mbs| mbs.checked_mul(16))
            .ok_or_else(geometry_limit)?;
        let coded_height = sps
            .pic_height_in_map_units_minus1
            .checked_add(1)
            .and_then(|mbs| mbs.checked_mul(16))
            .ok_or_else(geometry_limit)?;
        let vui = sps.vui.as_ref();
        if coded_width > 4096
            || coded_height > 4096
            || coded_width
                .checked_mul(coded_height)
                .is_none_or(|area| area > MAX_PIXELS)
            || sps.max_num_ref_frames > 16
            || vui
                .and_then(|v| v.bitstream_restriction.as_ref())
                .is_some_and(|r| r.max_num_reorder_frames > 16 || r.max_dec_frame_buffering > 16)
        {
            return Err(geometry_limit());
        }
        let (width, height) = sps.cropped_dimensions();
        if width == 0 || height == 0 {
            return Err(invalid("invalid AVC frame cropping"));
        }
        let pps =
            Pps::parse(&parse_nal_unit(&config.pps[0]).map_err(invalid)?.rbsp).map_err(invalid)?;
        if pps.num_slice_groups_minus1 != 0 {
            return Err(unsupported("AVC video with multiple slice groups"));
        }
        let aspect = vui
            .and_then(|v| v.aspect_ratio.as_ref())
            .map_or((1, 1), |info| {
                const ASPECT: [(u16, u16); 17] = [
                    (1, 1),
                    (1, 1),
                    (12, 11),
                    (10, 11),
                    (16, 11),
                    (40, 33),
                    (24, 11),
                    (20, 11),
                    (32, 11),
                    (80, 33),
                    (18, 11),
                    (15, 11),
                    (64, 33),
                    (160, 99),
                    (4, 3),
                    (3, 2),
                    (2, 1),
                ];
                info.extended_sar
                    .or_else(|| ASPECT.get(usize::from(info.aspect_ratio_idc)).copied())
                    .filter(|&(w, h)| w != 0 && h != 0)
                    .unwrap_or((1, 1))
            });
        let aspect = match (
            params.options.get("pasp_h_spacing"),
            params.options.get("pasp_v_spacing"),
        ) {
            (Some(w), Some(h)) => (w.parse().map_err(invalid)?, h.parse().map_err(invalid)?),
            (None, None) => (u32::from(aspect.0), u32::from(aspect.1)),
            _ => return Err(invalid("incomplete MP4 video pixel aspect ratio")),
        };
        if aspect.0 == 0 || aspect.1 == 0 {
            return Err(invalid("invalid MP4 video pixel aspect ratio"));
        }
        if params.options.get("clap_width").is_some() {
            return Err(unsupported("MP4 video clean aperture"));
        }
        let signal = vui.and_then(|v| v.video_signal_type.as_ref());
        let transfer = params
            .options
            .get("colr_transfer")
            .map(str::parse::<u16>)
            .transpose()
            .map_err(invalid)?;
        let coded_transfer = signal
            .and_then(|s| s.colour_description.as_ref())
            .map(|description| u16::from(description.transfer_characteristics));
        if [transfer, coded_transfer]
            .into_iter()
            .flatten()
            .any(|code| matches!(code, 16 | 18))
        {
            return Err(unsupported("embedded AVC HDR transfer characteristics"));
        }
        let (full_range, matrix) = match params.options.get("colr_type") {
            None => (
                signal.is_some_and(|signal| signal.video_full_range_flag),
                signal
                    .and_then(|s| s.colour_description.as_ref())
                    .map_or(2, |c| u16::from(c.matrix_coefficients)),
            ),
            Some("nclx") => (
                params.options.get("colr_full_range") == Some("1"),
                params
                    .options
                    .get("colr_matrix")
                    .ok_or_else(|| invalid("missing MP4 video colour matrix"))?
                    .parse()
                    .map_err(invalid)?,
            ),
            Some(_) => return Err(unsupported("unsupported MP4 video colour information")),
        };
        let (kr, kb) = match matrix {
            1 => (0.2126, 0.0722),
            2 | 5 | 6 => (0.299, 0.114),
            9 => (0.2627, 0.0593),
            _ => return Err(unsupported("unsupported AVC video colour matrix")),
        };
        Ok((
            Self {
                width,
                height,
                aspect,
                full_range,
                kr,
                kb,
            },
            config.length_size,
            pixel_format,
        ))
    }

    fn avc_image(
        &self,
        frame: VideoFrame,
        pixel_format: PixelFormat,
        start_us: u64,
        duration_us: u64,
    ) -> Result<VideoImage, EngineError> {
        let [y, u, v] = frame.image_planes() else {
            return Err(invalid("AVC video frame requires three image planes"));
        };
        let picture = Yuv {
            planes: [y, u, v].map(|plane| Plane {
                bytes: &plane.data,
                stride: plane.stride,
            }),
            x: 0,
            y: 0,
            bottom_up: false,
        };
        use PixelFormat::*;
        match pixel_format {
            Yuv420P => self.image::<2, 2>(picture, start_us, duration_us),
            Yuv422P => self.image::<2, 1>(picture, start_us, duration_us),
            Yuv444P => self.image::<1, 1>(picture, start_us, duration_us),
            Yuv420P10Le => self.image_depth::<2, 2, 2>(picture, 10, start_us, duration_us),
            Yuv422P10Le => self.image_depth::<2, 1, 2>(picture, 10, start_us, duration_us),
            Yuv444P10Le => self.image_depth::<1, 1, 2>(picture, 10, start_us, duration_us),
            _ => Err(unsupported("unsupported AVC image planes")),
        }
    }

    fn image<const SX: usize, const SY: usize>(
        &self,
        picture: Yuv<'_>,
        start_us: u64,
        duration_us: u64,
    ) -> Result<VideoImage, EngineError> {
        self.image_depth::<SX, SY, 1>(picture, 8, start_us, duration_us)
    }

    fn image_depth<const SX: usize, const SY: usize, const BYTES: usize>(
        &self,
        picture: Yuv<'_>,
        depth: u8,
        start_us: u64,
        duration_us: u64,
    ) -> Result<VideoImage, EngineError> {
        let (width, height) = (self.width as usize, self.height as usize);
        let mut rgb = Vec::with_capacity(width * height * 3);
        let unit = f32::from(1u16 << (depth - 8));
        let neutral = 128.0 * unit;
        let (black, y_scale, uv_scale) = if self.full_range {
            let full = 255.0 / f32::from((1u16 << depth) - 1);
            (0.0, full, full)
        } else {
            (16.0 * unit, 255.0 / (219.0 * unit), 255.0 / (224.0 * unit))
        };
        let kg = 1.0 - self.kr - self.kb;
        let red = 2.0 * (1.0 - self.kr) * uv_scale;
        let blue = 2.0 * (1.0 - self.kb) * uv_scale;
        let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        let sample = |line: &[u8], col: usize| {
            if BYTES == 1 {
                f32::from(line[col])
            } else {
                f32::from(u16::from_le_bytes([line[2 * col], line[2 * col + 1]]))
            }
        };
        let [y, u, v] = picture.planes;
        let chroma_start = picture.x / SX;
        let chroma_width = (picture.x % SX + width).div_ceil(SX);
        for row in 0..height {
            let row = picture.y
                + if picture.bottom_up {
                    height - 1 - row
                } else {
                    row
                };
            let y_line = y.line(row, picture.x * BYTES, width * BYTES)?;
            let u_line = u.line(row / SY, chroma_start * BYTES, chroma_width * BYTES)?;
            let v_line = v.line(row / SY, chroma_start * BYTES, chroma_width * BYTES)?;
            for i in 0..width {
                let luma = (sample(y_line, i) - black) * y_scale;
                let chroma = (picture.x % SX + i) / SX;
                let r = red * (sample(v_line, chroma) - neutral);
                let b = blue * (sample(u_line, chroma) - neutral);
                rgb.extend([
                    byte(luma + r),
                    byte(luma - (self.kr * r + self.kb * b) / kg),
                    byte(luma + b),
                ]);
            }
        }
        Ok(VideoImage {
            rgb,
            width: self.width,
            height: self.height,
            aspect: self.aspect,
            start_us,
            duration_us,
        })
    }
}

struct Yuv<'a> {
    planes: [Plane<'a>; 3],
    x: usize,
    y: usize,
    bottom_up: bool,
}

struct Plane<'a> {
    bytes: &'a [u8],
    stride: usize,
}

impl<'a> Plane<'a> {
    fn line(&self, row: usize, left: usize, width: usize) -> Result<&'a [u8], EngineError> {
        if left
            .checked_add(width)
            .is_none_or(|right| right > self.stride)
        {
            return Err(invalid(
                "video plane stride is smaller than its picture window",
            ));
        }
        row.checked_mul(self.stride)
            .and_then(|start| self.bytes.get(start.checked_add(left)?..)?.get(..width))
            .ok_or_else(|| invalid("truncated video plane"))
    }
}
