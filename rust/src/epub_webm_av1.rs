//! Preflight AV1 temporal units and drive the bounded native decoder.
use oxideav_av1::{
    codec_config::Av1CodecConfig,
    decoder::SpecDecodeSession,
    obu::{parse_obu, ObuType},
    sequence_header::{parse_sequence_header, SequenceHeader},
};
use oxideav_core::{Packet, PixelFormat, VideoFrame, VideoPlane};
use oxideav_mkv::demux::MatrixCoefficients;

use super::{check_size, invalid, limited, unsupported, EngineError, VideoColour, MAX_PIXELS};

const MAX_PRIVATE_BYTES: usize = 64 * 1024;
const MAX_BLOCK_OBUS: usize = 128;
const MAX_FRAMES_PER_BLOCK: usize = 8;
const MAX_OBUS: usize = 500_000;

pub(super) struct Track {
    sequence: Option<SequenceHeader>,
    bootstrap: Vec<u8>,
    coded: (u32, u32),
    obus: usize,
}

pub(super) struct Playback {
    session: SpecDecodeSession,
    coded: (u32, u32),
    format: PixelFormat,
    depth: u8,
}

impl Track {
    pub(super) fn new(bytes: &[u8], coded: (u32, u32)) -> Result<Self, EngineError> {
        if bytes.len() > MAX_PRIVATE_BYTES {
            return Err(limited("WebM AV1 codec configuration exceeds 64 KiB"));
        }
        let mut track = Self {
            sequence: None,
            bootstrap: Vec::new(),
            coded,
            obus: 0,
        };
        if !bytes.is_empty() {
            let config = Av1CodecConfig::parse(bytes)
                .ok_or_else(|| invalid("invalid WebM AV1 codec configuration"))?;
            track.scan(&config.config_obus, true)?;
        }
        Ok(track)
    }

    pub(super) fn inspect(&mut self, bytes: &[u8]) -> Result<(bool, bool), EngineError> {
        self.scan(bytes, false)
    }

    fn scan(&mut self, mut bytes: &[u8], configuration: bool) -> Result<(bool, bool), EngineError> {
        let mut frames = 0;
        let mut shown = 0;
        let mut key = false;
        let mut count = 0;
        while !bytes.is_empty() {
            count += 1;
            self.obus += 1;
            if count > MAX_BLOCK_OBUS || self.obus > MAX_OBUS {
                return Err(limited("WebM AV1 exceeds its OBU count budget"));
            }
            let (obu, used) = parse_obu(bytes).map_err(invalid)?;
            if obu.spatial_id != 0 || obu.temporal_id != 0 {
                return Err(unsupported("layered WebM AV1"));
            }
            match obu.obu_type {
                ObuType::SequenceHeader => self.sequence(&bytes[..used], obu.payload)?,
                ObuType::Frame | ObuType::FrameHeader if !configuration => {
                    let sequence = self
                        .sequence
                        .as_ref()
                        .ok_or_else(|| invalid("missing WebM AV1 sequence header"))?;
                    let (independent, visible) = show_frame(sequence, obu.payload)?;
                    key |= frames == 0 && independent;
                    frames += 1;
                    shown += usize::from(visible);
                    if frames > MAX_FRAMES_PER_BLOCK {
                        return Err(limited(
                            "WebM AV1 exceeds its coded frames per block budget",
                        ));
                    }
                    if shown > 1 {
                        return Err(unsupported(
                            "multiple WebM AV1 pictures at one block timestamp",
                        ));
                    }
                }
                ObuType::Metadata | ObuType::TemporalDelimiter | ObuType::Padding => {}
                ObuType::TileGroup | ObuType::RedundantFrameHeader if !configuration => {}
                _ => return Err(unsupported("WebM AV1 OBU type in this packet")),
            }
            bytes = &bytes[used..];
        }
        Ok((key, shown == 1))
    }

    fn sequence(&mut self, encoded: &[u8], payload: &[u8]) -> Result<(), EngineError> {
        if encoded.len() > MAX_PRIVATE_BYTES {
            return Err(limited("WebM AV1 sequence header exceeds 64 KiB"));
        }
        let sequence = parse_sequence_header(payload).map_err(invalid)?;
        let maximum = (
            sequence.max_frame_width_minus_1 + 1,
            sequence.max_frame_height_minus_1 + 1,
        );
        check_size(maximum)?;
        if maximum != self.coded {
            return Err(unsupported(
                "WebM AV1 sequence dimensions disagree with its track",
            ));
        }
        if sequence.operating_points_cnt_minus_1 != 0 {
            return Err(unsupported("layered WebM AV1"));
        }
        let signal = &sequence.color_config;
        if matches!(signal.transfer_characteristics, 16 | 18) {
            return Err(unsupported("WebM AV1 HDR transfer characteristic"));
        }
        matrix(signal.matrix_coefficients)?;
        if let Some(previous) = &self.sequence {
            if previous != &sequence {
                return Err(unsupported("WebM AV1 changes its sequence header"));
            }
        } else {
            self.sequence = Some(sequence);
            self.bootstrap = encoded.to_vec();
        }
        Ok(())
    }

    pub(super) fn colour(&self) -> Result<VideoColour, EngineError> {
        let signal = &self
            .sequence
            .as_ref()
            .ok_or_else(|| invalid("missing WebM AV1 sequence header"))?
            .color_config;
        Ok(VideoColour {
            depth: signal.bit_depth,
            matrix: matrix(signal.matrix_coefficients)?,
            full: signal.color_range,
        })
    }

    pub(super) fn into_playback(self) -> Result<Playback, EngineError> {
        let sequence = self
            .sequence
            .ok_or_else(|| invalid("missing WebM AV1 sequence header"))?;
        let signal = sequence.color_config;
        let format = match (
            signal.mono_chrome,
            signal.bit_depth,
            signal.subsampling_x,
            signal.subsampling_y,
        ) {
            (true, 8, true, true) => PixelFormat::Gray8,
            (true, 10, true, true) => PixelFormat::Gray10Le,
            (true, 12, true, true) => PixelFormat::Gray12Le,
            (false, 8, true, true) => PixelFormat::Yuv420P,
            (false, 8, true, false) => PixelFormat::Yuv422P,
            (false, 8, false, false) => PixelFormat::Yuv444P,
            (false, 10, true, true) => PixelFormat::Yuv420P10Le,
            (false, 10, true, false) => PixelFormat::Yuv422P10Le,
            (false, 10, false, false) => PixelFormat::Yuv444P10Le,
            (false, 12, true, true) => PixelFormat::Yuv420P12Le,
            (false, 12, true, false) => PixelFormat::Yuv422P12Le,
            (false, 12, false, false) => PixelFormat::Yuv444P12Le,
            _ => return Err(unsupported("WebM AV1 chroma sampling or depth")),
        };
        let mut session = SpecDecodeSession::new();
        session.set_max_picture_size(MAX_PIXELS);
        let startup = session
            .decode_temporal_unit(&self.bootstrap)
            .map_err(decoder_error)?;
        if !startup.is_empty() {
            return Err(invalid("unexpected WebM AV1 picture in sequence header"));
        }
        Ok(Playback {
            session,
            coded: self.coded,
            format,
            depth: signal.bit_depth,
        })
    }
}

impl Playback {
    pub(super) fn next(
        &mut self,
        packet: &Packet,
    ) -> Result<Option<(VideoFrame, PixelFormat)>, EngineError> {
        let mut frames = self
            .session
            .decode_temporal_unit(&packet.data)
            .map_err(decoder_error)?;
        if frames.len() > 1 {
            return Err(invalid(
                "AV1 decoder returned multiple WebM pictures for one block",
            ));
        }
        let Some(frame) = frames.pop() else {
            return Ok(None);
        };
        let planes = self.format.plane_count();
        if (frame.width, frame.height) != self.coded
            || frame.bit_depth != self.depth
            || frame.planes.len() != planes
            || frame.plane_dims.len() != planes
            || (planes == 1 && frame.plane_dims[0] != self.coded)
        {
            return Err(unsupported(
                "WebM AV1 changes decoded picture geometry or depth",
            ));
        }
        let bytes = if frame.bit_depth > 8 { 2 } else { 1 };
        let planes = frame
            .planes
            .into_iter()
            .zip(frame.plane_dims)
            .map(|(data, (width, _))| VideoPlane {
                stride: width as usize * bytes,
                data,
            })
            .collect();
        Ok(Some((
            VideoFrame {
                pts: packet.pts,
                planes,
            },
            self.format,
        )))
    }
}

// These are the first fields in uncompressed_header(). The sequence header
// determines whether the fields are implicit for a reduced still picture.
fn show_frame(sequence: &SequenceHeader, payload: &[u8]) -> Result<(bool, bool), EngineError> {
    if sequence.reduced_still_picture_header {
        return Ok((true, true));
    }
    let first = *payload
        .first()
        .ok_or_else(|| invalid("empty WebM AV1 frame header"))?;
    if first & 0x80 != 0 {
        return Ok((false, true)); // show_existing_frame
    }
    let visible = first & 0x10 != 0;
    let key = (first >> 5) & 3 == 0;
    // A displayed keyframe resets all reference slots and can start a seek.
    Ok((key && visible, visible))
}

fn matrix(code: u8) -> Result<MatrixCoefficients, EngineError> {
    match code {
        1 => Ok(MatrixCoefficients::BT709),
        2 => Ok(MatrixCoefficients::Unspecified),
        4 => Ok(MatrixCoefficients::UsFcc73682),
        5 => Ok(MatrixCoefficients::BT470Bg),
        6 => Ok(MatrixCoefficients::Smpte170M),
        7 => Ok(MatrixCoefficients::Smpte240M),
        9 => Ok(MatrixCoefficients::BT2020NonConstantLuminance),
        _ => Err(unsupported("WebM AV1 colour matrix")),
    }
}

fn decoder_error(error: oxideav_av1::Error) -> EngineError {
    match error {
        oxideav_av1::Error::PictureSizeExceedsLimit => {
            limited("WebM AV1 coded picture exceeds limits")
        }
        _ => invalid(error),
    }
}
