//! Decode bounded AV1, VP8 and VP9 pictures using WebM presentation timestamps.
use std::{ops::Range, sync::Mutex};

use oxideav_core::{
    CodecId, Decoder, Demuxer, Error, Frame, MediaType, Packet, PixelFormat, VideoFrame,
};
use oxideav_mkv::demux::{
    AlphaMode, ColourRange, FlagInterlaced, MatrixCoefficients, Primaries, TransferCharacteristics,
};
use oxideav_vp8::{parse_header, Vp8Decoder};
use oxideav_vp9::{
    parse_uncompressed_header_with_refs, split_superframe, ColorConfig, ColorSpace, FrameType,
    RefFrameState, Vp9Decoder,
};

use super::{
    invalid, limited, unsupported, EngineError, Geometry, Plane, VideoImage, Yuv,
    MAX_PACKETS_PER_FRAME, MAX_PIXELS,
};
use crate::epub::webm;

#[path = "epub_webm_av1.rs"]
mod av1;

enum Codec {
    Vp8,
    Vp9(Option<ColorConfig>),
    Av1(Box<av1::Track>),
}

enum Playback {
    Native(Mutex<Box<dyn Decoder>>),
    Av1(Box<av1::Playback>),
}

#[derive(Clone, Copy)]
struct VideoColour {
    depth: u8,
    matrix: MatrixCoefficients,
    full: bool,
}

struct Picture {
    packet: Packet,
    keyframe: bool,
    display: Option<Range<u64>>,
}

pub(crate) struct WebmSource {
    pictures: Vec<Picture>,
    next: usize,
    decoder: Playback,
    clip: Range<u64>,
    pub(super) geometry: Geometry,
    coded: (u32, u32),
    crop: (usize, usize),
}

impl WebmSource {
    pub(super) fn open(
        bytes: Vec<u8>,
        start: u64,
        end: Option<u64>,
        video_track: Option<usize>,
        names: &[String],
    ) -> Result<Self, EngineError> {
        let mut reader = webm::Reader::open(bytes)?;
        let index = reader.track(MediaType::Video, video_track, names)?;
        let stream = &reader.demux.streams()[index as usize];
        let coded = stream
            .params
            .width
            .zip(stream.params.height)
            .ok_or_else(|| invalid("missing WebM video dimensions"))?;
        check_size(coded)?;
        let mut codec = match stream.params.codec_id.as_str() {
            "vp8" => Codec::Vp8,
            "vp9" => Codec::Vp9(None),
            "av1" => Codec::Av1(Box::new(av1::Track::new(&stream.params.extradata, coded)?)),
            _ => return Err(unsupported("WebM video requires an AV1, VP8 or VP9 track")),
        };
        let timing = reader.demux.track_timing(index);
        let nominal = timing
            .and_then(|timing| timing.default_duration())
            .filter(|&n| n != 0);
        let duration = stream
            .duration
            .map(|ticks| webm::timestamp(stream.time_base, ticks))
            .transpose()?
            .map(u64::try_from)
            .transpose()
            .map_err(invalid)?;
        let (mut geometry, crop) = geometry(&reader, index, coded)?;
        let mut pictures = Vec::new();
        let mut shown = Vec::new();
        while let Some(unit) = reader.next()? {
            if unit.packet.stream_index != index {
                continue;
            }
            if unit.block_count != 1 || unit.discard_padding != 0 {
                return Err(unsupported("laced or padded WebM video blocks"));
            }
            let time = unit.time()?;
            let (keyframe, visible) = codec.inspect(&unit.packet.data, coded)?;
            if pictures.is_empty() && !keyframe {
                return Err(invalid("WebM video starts without a keyframe"));
            }
            if visible {
                let time = u64::try_from(time).map_err(invalid)?;
                if shown
                    .last()
                    .is_some_and(|&(_, previous, _, _)| previous >= time)
                {
                    return Err(invalid("WebM picture timestamps are out of order"));
                }
                let explicit = unit
                    .packet
                    .duration
                    .map(|duration| webm::timestamp(unit.packet.time_base, duration))
                    .transpose()?
                    .map(u64::try_from)
                    .transpose()
                    .map_err(invalid)?;
                let tick =
                    u64::try_from(webm::timestamp(unit.packet.time_base, 1)?).map_err(invalid)?;
                shown.push((pictures.len(), time, explicit, tick));
            }
            pictures.push(Picture {
                packet: unit.packet,
                keyframe,
                display: None,
            });
        }
        if shown.is_empty() {
            return Err(invalid("WebM has no displayed pictures"));
        }
        let (kr, kb, full) = colour(&reader, index, codec.colour()?)?;
        (geometry.kr, geometry.kb, geometry.full_range) = (kr, kb, full);
        let mut last_end = 0;
        for (i, &(packet, begin, explicit, tick)) in shown.iter().enumerate() {
            let following = shown.get(i + 1).map(|&(_, begin, _, _)| begin);
            let end = picture_end(begin, following, explicit, nominal, tick, duration)?;
            let interval = begin / 1000..end / 1000;
            if interval.is_empty() {
                return Err(invalid("WebM picture has no presentation duration"));
            }
            last_end = interval.end;
            pictures[packet].display = Some(interval);
        }
        let length = duration.map_or(last_end, |ns| last_end.max(ns / 1000));
        let clip = start..end.unwrap_or(length).min(length);
        if clip.is_empty() {
            return Err(invalid("WebM video clip lies outside its presentation"));
        }
        let first = shown.partition_point(|&(packet, _, _, _)| {
            pictures[packet]
                .display
                .as_ref()
                .is_some_and(|time| time.end <= start)
        });
        let next = if let Some(&(packet, _, _, _)) = shown.get(first) {
            let key = pictures[..=packet]
                .iter()
                .rposition(|frame| frame.keyframe)
                .ok_or_else(|| invalid("WebM seek has no earlier keyframe"))?;
            if packet - key >= MAX_PACKETS_PER_FRAME {
                return Err(limited("WebM seek exceeds its keyframe preroll budget"));
            }
            key
        } else {
            pictures.len()
        };
        let decoder = match codec {
            Codec::Vp8 => {
                Playback::Native(Mutex::new(Box::new(Vp8Decoder::new(CodecId::new("vp8")))))
            }
            Codec::Vp9(_) => Playback::Native(Mutex::new(Box::new(Vp9Decoder::new()))),
            Codec::Av1(track) => Playback::Av1(Box::new(track.into_playback()?)),
        };
        Ok(Self {
            pictures,
            next,
            decoder,
            clip,
            geometry,
            coded,
            crop,
        })
    }

    pub(super) fn next_image(&mut self) -> Result<Option<VideoImage>, EngineError> {
        for _ in 0..MAX_PACKETS_PER_FRAME {
            let Some(picture) = self.pictures.get(self.next) else {
                return Ok(None);
            };
            if picture
                .display
                .as_ref()
                .is_some_and(|time| time.start >= self.clip.end)
            {
                return Ok(None);
            }
            self.next += 1;
            let frame = self.decoder.next(&picture.packet, self.coded)?;
            match (frame, &picture.display) {
                (None, None) => continue,
                (Some((frame, format)), Some(time)) => {
                    if time.end <= self.clip.start {
                        continue;
                    }
                    let start = time.start.max(self.clip.start);
                    let stop = time.end.min(self.clip.end);
                    return self.image(&frame, format, start, stop - start).map(Some);
                }
                _ => return Err(invalid("WebM decoder disagrees with the picture timeline")),
            }
        }
        Err(limited(
            "WebM video exceeds its per-picture decoding budget",
        ))
    }

    fn image(
        &self,
        frame: &VideoFrame,
        format: Option<PixelFormat>,
        start: u64,
        duration: u64,
    ) -> Result<VideoImage, EngineError> {
        let gray_depth = match format {
            Some(PixelFormat::Gray8) => Some(8),
            Some(PixelFormat::Gray10Le) => Some(10),
            Some(PixelFormat::Gray12Le) => Some(12),
            _ => None,
        };
        if let Some(depth) = gray_depth {
            let [luma] = frame.planes.as_slice() else {
                return Err(invalid(
                    "WebM decoder returned unexpected monochrome picture planes",
                ));
            };
            let plane = Plane {
                bytes: &luma.data,
                stride: luma.stride,
            };
            return if depth == 8 {
                self.geometry
                    .gray_image::<1>(plane, self.crop, depth, start, duration)
            } else {
                self.geometry
                    .gray_image::<2>(plane, self.crop, depth, start, duration)
            };
        }
        let [first, second, third] = frame.planes.as_slice() else {
            return Err(invalid("WebM decoder returned unexpected picture planes"));
        };
        let planes = [first, second, third].map(|plane| Plane {
            bytes: &plane.data,
            stride: plane.stride,
        });
        match format {
            Some(PixelFormat::Gbrp8) => {
                return self
                    .geometry
                    .gbr_image::<1>(planes, self.crop, 8, start, duration);
            }
            Some(PixelFormat::Gbrp10Le) => {
                return self
                    .geometry
                    .gbr_image::<2>(planes, self.crop, 10, start, duration);
            }
            Some(PixelFormat::Gbrp12Le) => {
                return self
                    .geometry
                    .gbr_image::<2>(planes, self.crop, 12, start, duration);
            }
            _ => {}
        }
        let picture = Yuv {
            planes,
            x: self.crop.0,
            y: self.crop.1,
            bottom_up: false,
        };
        match format {
            Some(PixelFormat::Yuv420P) => self.geometry.image::<2, 2>(picture, start, duration),
            Some(PixelFormat::Yuv422P) => self.geometry.image::<2, 1>(picture, start, duration),
            Some(PixelFormat::Yuv444P) => self.geometry.image::<1, 1>(picture, start, duration),
            Some(PixelFormat::Yuv440P) => self.geometry.image::<1, 2>(picture, start, duration),
            Some(PixelFormat::Yuv420P10Le) => self
                .geometry
                .image_depth::<2, 2, 2>(picture, 10, start, duration),
            Some(PixelFormat::Yuv422P10Le) => self
                .geometry
                .image_depth::<2, 1, 2>(picture, 10, start, duration),
            Some(PixelFormat::Yuv444P10Le) => self
                .geometry
                .image_depth::<1, 1, 2>(picture, 10, start, duration),
            Some(PixelFormat::Yuv440P10Le) => self
                .geometry
                .image_depth::<1, 2, 2>(picture, 10, start, duration),
            Some(PixelFormat::Yuv420P12Le) => self
                .geometry
                .image_depth::<2, 2, 2>(picture, 12, start, duration),
            Some(PixelFormat::Yuv422P12Le) => self
                .geometry
                .image_depth::<2, 1, 2>(picture, 12, start, duration),
            Some(PixelFormat::Yuv444P12Le) => self
                .geometry
                .image_depth::<1, 1, 2>(picture, 12, start, duration),
            Some(PixelFormat::Yuv440P12Le) => self
                .geometry
                .image_depth::<1, 2, 2>(picture, 12, start, duration),
            _ => Err(unsupported(
                "WebM video requires planar YUV, monochrome or RGB pictures",
            )),
        }
    }
}

impl Playback {
    fn next(
        &mut self,
        packet: &Packet,
        coded: (u32, u32),
    ) -> Result<Option<(VideoFrame, Option<PixelFormat>)>, EngineError> {
        match self {
            Self::Native(decoder) => {
                let decoder = decoder.get_mut().map_err(invalid)?;
                decoder.send_packet(packet).map_err(invalid)?;
                let result = match decoder.receive_frame() {
                    Ok(Frame::StreamParams(..)) => decoder.receive_frame(),
                    frame => frame,
                };
                match result {
                    Err(Error::NeedMore) => Ok(None),
                    Err(error) => Err(invalid(error)),
                    Ok(Frame::Video(frame)) => {
                        let params = decoder
                            .output_params()
                            .ok_or_else(|| invalid("missing WebM picture format"))?;
                        if params.width.zip(params.height) != Some(coded) || frame.pts != packet.pts
                        {
                            return Err(invalid(
                                "WebM decoder returned unexpected picture dimensions or timestamp",
                            ));
                        }
                        Ok(Some((frame, params.pixel_format)))
                    }
                    _ => Err(invalid("WebM decoder returned a non-video frame")),
                }
            }
            Self::Av1(decoder) => decoder
                .next(packet)
                .map(|frame| frame.map(|(frame, fmt)| (frame, Some(fmt)))),
        }
    }
}

impl Codec {
    fn inspect(&mut self, bytes: &[u8], coded: (u32, u32)) -> Result<(bool, bool), EngineError> {
        match self {
            Self::Av1(state) => state.inspect(bytes),
            Self::Vp8 => {
                let header = parse_header(bytes).map_err(invalid)?;
                if let Some(key) = header.keyframe {
                    let size = (u32::from(key.width), u32::from(key.height));
                    check_size(size)?;
                    if size != coded {
                        return Err(unsupported("WebM VP8 changes coded picture dimensions"));
                    }
                }
                Ok((header.keyframe.is_some(), header.tag.show_frame))
            }
            Self::Vp9(colour) => {
                let dimensions = [coded; 8];
                let mut keyframe = false;
                let mut shown = 0;
                for (index, bytes) in split_superframe(bytes).into_iter().enumerate() {
                    let references = colour.map(|color_config| RefFrameState {
                        ref_dims: &dimensions,
                        color_config,
                    });
                    let header =
                        parse_uncompressed_header_with_refs(bytes, references).map_err(invalid)?;
                    if !header.show_existing_frame {
                        let size = (header.frame_width, header.frame_height);
                        check_size(size)?;
                        if size != coded {
                            return Err(unsupported("WebM VP9 changes coded picture dimensions"));
                        }
                        let signal = header.color_config;
                        if signal.color_space == ColorSpace::Rgb
                            || signal.color_space == ColorSpace::Reserved
                        {
                            return Err(unsupported("WebM VP9 RGB or reserved colour space"));
                        }
                        if *colour.get_or_insert(signal) != signal {
                            return Err(unsupported("WebM VP9 changes its colour configuration"));
                        }
                        keyframe |= index == 0 && header.frame_type == FrameType::KeyFrame;
                    }
                    shown += usize::from(header.show_frame || header.show_existing_frame);
                }
                if shown > 1 {
                    return Err(unsupported(
                        "multiple WebM VP9 pictures at one block timestamp",
                    ));
                }
                Ok((keyframe, shown == 1))
            }
        }
    }

    fn colour(&self) -> Result<VideoColour, EngineError> {
        match self {
            Self::Av1(track) => track.colour(),
            Self::Vp9(Some(colour)) => Ok(VideoColour {
                depth: colour.bit_depth,
                matrix: match colour.color_space {
                    ColorSpace::Bt709 => MatrixCoefficients::BT709,
                    ColorSpace::Smpte240 => MatrixCoefficients::Smpte240M,
                    ColorSpace::Bt2020 => MatrixCoefficients::BT2020NonConstantLuminance,
                    _ => MatrixCoefficients::Unspecified,
                },
                full: colour.color_range_full,
            }),
            _ => Ok(VideoColour {
                depth: 8,
                matrix: MatrixCoefficients::Unspecified,
                full: false,
            }),
        }
    }
}

fn check_size((width, height): (u32, u32)) -> Result<(), EngineError> {
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || width
            .checked_mul(height)
            .is_none_or(|area| area > MAX_PIXELS)
    {
        return Err(limited("WebM coded picture geometry exceeds limits"));
    }
    Ok(())
}

fn picture_end(
    begin: u64,
    following: Option<u64>,
    duration: Option<u64>,
    nominal: Option<u64>,
    tick: u64,
    total: Option<u64>,
) -> Result<u64, EngineError> {
    let known = duration
        .or(nominal)
        .and_then(|span| begin.checked_add(span));
    let end = match (following, known) {
        (Some(next), Some(known)) if duration.is_some() || known.saturating_add(tick) < next => {
            known.min(next)
        }
        (Some(next), _) => next,
        (None, Some(known)) => known,
        (None, None) => total.ok_or_else(|| unsupported("WebM has no final picture duration"))?,
    };
    if end <= begin || end > webm::MAX_TIME_NS {
        return Err(invalid("invalid WebM picture duration"));
    }
    Ok(end)
}

fn geometry(
    reader: &webm::Reader,
    index: u32,
    coded: (u32, u32),
) -> Result<(Geometry, (usize, usize)), EngineError> {
    let demux = &reader.demux;
    if demux
        .video_alpha_mode(index)
        .is_some_and(|mode| mode != AlphaMode::None)
        || demux.video_projection(index).is_some()
        || demux.video_interlacing(index).is_some_and(|format| {
            !matches!(
                format.flag(),
                FlagInterlaced::Undetermined | FlagInterlaced::Progressive
            )
        })
    {
        return Err(unsupported("WebM alpha, projection or interlaced video"));
    }
    let (mut width, mut height) = coded;
    let mut crop = (0, 0);
    let mut aspect = (1, 1);
    if let Some(value) = demux.video_geometry(index) {
        let [left, right, top, bottom] = [
            value.pixel_crop_left(),
            value.pixel_crop_right(),
            value.pixel_crop_top(),
            value.pixel_crop_bottom(),
        ];
        let cropped = |pixels: u32, front: u64, back: u64| {
            u64::from(pixels)
                .checked_sub(front)?
                .checked_sub(back)
                .filter(|&n| n != 0)
        };
        width = cropped(width, left, right)
            .ok_or_else(|| invalid("invalid WebM horizontal crop"))? as u32;
        height = cropped(height, top, bottom)
            .ok_or_else(|| invalid("invalid WebM vertical crop"))? as u32;
        crop = (left as usize, top as usize);
        let w = value.display_width().unwrap_or(u64::from(width));
        let h = value.display_height().unwrap_or(u64::from(height));
        let x = w
            .checked_mul(u64::from(height))
            .filter(|&n| n != 0)
            .ok_or_else(|| invalid("invalid WebM display width"))?;
        let y = h
            .checked_mul(u64::from(width))
            .filter(|&n| n != 0)
            .ok_or_else(|| invalid("invalid WebM display height"))?;
        let (mut a, mut b) = (x, y);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        aspect = (
            u32::try_from(x / a).map_err(invalid)?,
            u32::try_from(y / a).map_err(invalid)?,
        );
    }
    Ok((
        Geometry {
            width,
            height,
            aspect,
            full_range: false,
            kr: 0.299,
            kb: 0.114,
            region: None,
        },
        crop,
    ))
}

fn colour(
    reader: &webm::Reader,
    index: u32,
    signal: VideoColour,
) -> Result<(f32, f32, bool), EngineError> {
    let declared = reader.demux.video_colour(index);
    if declared.is_some_and(|colour| {
        (colour.bits_per_channel() != 0 && colour.bits_per_channel() != u64::from(signal.depth))
            || matches!(
                colour.transfer_characteristics(),
                TransferCharacteristics::BT2100Pq | TransferCharacteristics::AribStdB67Hlg
            )
    }) {
        return Err(unsupported("WebM HDR colour or inconsistent sample depth"));
    }
    if signal.matrix == MatrixCoefficients::Identity {
        if declared.is_some_and(|colour| {
            !matches!(
                colour.matrix_coefficients(),
                MatrixCoefficients::Unspecified | MatrixCoefficients::Identity
            ) || !matches!(colour.range(), ColourRange::Unspecified | ColourRange::Full)
                || !matches!(
                    colour.primaries(),
                    Primaries::Unspecified | Primaries::BT709
                )
                || !matches!(
                    colour.transfer_characteristics(),
                    TransferCharacteristics::Unspecified | TransferCharacteristics::Iec61966_2_1
                )
        }) {
            return Err(unsupported(
                "WebM AV1 RGB container colour conflicts with its sequence",
            ));
        }
        // RGB bypasses the luma/chroma matrix.
        return Ok((0.2126, 0.0722, true));
    }
    let matrix = declared
        .map(|colour| colour.matrix_coefficients())
        .filter(|&matrix| matrix != MatrixCoefficients::Unspecified)
        .unwrap_or(signal.matrix);
    let (kr, kb) = match matrix {
        MatrixCoefficients::Unspecified
        | MatrixCoefficients::BT470Bg
        | MatrixCoefficients::Smpte170M => (0.299, 0.114),
        MatrixCoefficients::BT709 => (0.2126, 0.0722),
        MatrixCoefficients::Smpte240M => (0.212, 0.087),
        MatrixCoefficients::BT2020NonConstantLuminance => (0.2627, 0.0593),
        MatrixCoefficients::UsFcc73682 => (0.30, 0.11),
        _ => return Err(unsupported("WebM colour matrix")),
    };
    let full = match declared.map_or(ColourRange::Unspecified, |colour| colour.range()) {
        ColourRange::Unspecified => signal.full,
        ColourRange::Broadcast => false,
        ColourRange::Full => true,
        _ => return Err(unsupported("WebM colour range")),
    };
    Ok((kr, kb, full))
}
