//! Index validated Ogg Theora packets and decode the selected display intervals.
use std::ops::Range;

use oxideav_opus::OUTPUT_SAMPLE_RATE_HZ;
use oxideav_theora::{
    classify_packet, decode_identification_header, decode_setup_header,
    frame_index_from_granule_position, granule_position_from_legacy,
    keyframe_index_from_granule_position, parse_comment_header, ColorSpace, FrameDecoder,
    PixelFormat, TheoraIdentHeader, TheoraPacketKind,
};

use super::{
    invalid, limited, unsupported, EngineError, Geometry, Plane, VideoImage, Yuv, MAX_PIXELS,
};
use crate::epub::{
    audio::{ogg_mixed, opus, vorbis},
    media::SpatialFragment,
    ogg::{self, Codec, NamedTracks, Segment},
};

const MAX_PREROLL: usize = 512;
const MAX_HEADER_BYTES: usize = 1024 * 1024;
const MAX_DURATION_US: u128 = (1u128 << 53) - 1;

pub(crate) struct TheoraSource {
    links: Vec<Link>,
    current: Option<FrameDecoder>,
    link: usize,
    next: usize,
    clip: Range<u64>,
}

struct Link {
    ident: TheoraIdentHeader,
    geometry: Geometry,
    configuration: Vec<u8>,
    data: Vec<u8>,
    frames: Vec<FrameTime>,
    end_us: u64,
}

struct FrameTime {
    bytes: Range<usize>,
    interval: Range<u64>,
    keyframe: bool,
}

impl TheoraSource {
    pub(super) fn open(
        bytes: &[u8],
        start_us: u64,
        end_us: Option<u64>,
        names: &[String],
    ) -> Result<Self, EngineError> {
        let named = (!names.is_empty())
            .then(|| ogg::named_tracks(bytes, names))
            .transpose()?;
        let segments = ogg::groups(bytes, Codec::Theora, None, named.as_ref())?;
        let soundtrack = match named.as_ref() {
            Some(tracks) if segments.len() > 1 => tracks.audio_codec(),
            Some(_) => None,
            None => chained_soundtrack(&segments)?,
        };
        let audio: Option<(u32, Vec<_>)> =
            if let Some(tracks) = named.as_ref().filter(|tracks| tracks.mixed_audio()) {
                let track = ogg_mixed::Track::open(bytes, tracks)?;
                Some((track.spec().sample_rate, track.link_lengths().collect()))
            } else {
                match soundtrack {
                    Some(Codec::Opus) => Some((
                        OUTPUT_SAMPLE_RATE_HZ,
                        opus::Track::open(bytes, None, named.as_ref())?
                            .link_lengths()
                            .collect(),
                    )),
                    Some(Codec::Vorbis) => {
                        let track = vorbis::Track::open_ogg(bytes, named.as_ref())?;
                        Some((track.spec().sample_rate, track.link_lengths().collect()))
                    }
                    _ => None,
                }
            };
        let mut offset = 0;
        let mut clock = 0usize;
        let mut links = Vec::with_capacity(segments.len());
        for (index, segment) in segments.into_iter().enumerate() {
            let link = Link::index(segment, offset)?;
            if let Some((rate, sound)) = &audio {
                clock = clock
                    .checked_add(sound[index].max(link.sample_frames(*rate)?))
                    .ok_or_else(|| limited("Ogg chain duration exceeds limits"))?;
                offset = clock as u128 * 1_000_000_000 / u128::from(*rate);
            } else {
                offset += elapsed_ns(&link.ident, link.frames.len());
            }
            links.push(link);
        }
        let duration = micros(offset)?;
        let clip = start_us..end_us.unwrap_or(duration).min(duration);
        if clip.is_empty() {
            return Err(invalid("Ogg video clip lies outside its presentation"));
        }
        let link = links.partition_point(|link| link.end_us <= clip.start);
        let Some(frames) = links.get(link).map(|link| &link.frames) else {
            return Ok(Self {
                links,
                current: None,
                link,
                next: 0,
                clip,
            });
        };
        let first = frames.partition_point(|frame| frame.interval.end <= clip.start);
        let next = frames[..=first]
            .iter()
            .rposition(|frame| frame.keyframe)
            .ok_or_else(|| invalid("Ogg Theora clip has no preceding keyframe"))?;
        if first - next >= MAX_PREROLL {
            return Err(limited("Theora seek exceeds its keyframe preroll budget"));
        }
        Ok(Self {
            links,
            current: None,
            link,
            next,
            clip,
        })
    }

    pub(super) fn clip_spatial(&mut self, fragment: SpatialFragment) -> Result<(), EngineError> {
        for link in &mut self.links[self.link..] {
            if link.frames[0].interval.start >= self.clip.end {
                break;
            }
            link.geometry.clip_spatial(fragment)?;
        }
        Ok(())
    }

    pub(super) fn next_image(&mut self) -> Result<Option<VideoImage>, EngineError> {
        for _ in 0..MAX_PREROLL {
            let Some(link) = self.links.get(self.link) else {
                return Ok(None);
            };
            let Some(frame) = link.frames.get(self.next) else {
                self.link += 1;
                self.next = 0;
                self.current = None;
                continue;
            };
            if frame.interval.start >= self.clip.end {
                return Ok(None);
            }
            let decoder = if let Some(ref mut decoder) = self.current {
                decoder
            } else {
                let setup = decode_setup_header(&link.configuration).map_err(invalid)?;
                self.current
                    .insert(FrameDecoder::new(link.ident.clone(), setup).map_err(invalid)?)
            };
            decoder
                .decode_frame_in_place(&link.data[frame.bytes.clone()])
                .map_err(invalid)?;
            self.next += 1;
            if frame.interval.end <= self.clip.start {
                continue;
            }
            let start = frame.interval.start.max(self.clip.start);
            let stop = frame.interval.end.min(self.clip.end);
            return link.image(decoder, start, stop - start).map(Some);
        }
        Err(limited("Ogg video decode exceeds its per-frame work limit"))
    }
}

impl Link {
    fn index(segment: Segment<'_>, offset: u128) -> Result<Self, EngineError> {
        let mut packets = ogg::packets(segment.pages);
        let mut next_header = |kind| {
            let packet = packets
                .next()
                .transpose()?
                .ok_or_else(|| invalid("missing Ogg Theora headers"))?;
            if packet.data.len() > MAX_HEADER_BYTES {
                return Err(limited("Ogg Theora header exceeds 1 MiB"));
            }
            if classify_packet(&packet.data).map_err(invalid)? != kind
                || (packet.last_in_page() && packet.absgp_page() != 0)
            {
                return Err(invalid("invalid Ogg Theora header sequence"));
            }
            Ok(packet.data)
        };
        let ident =
            decode_identification_header(&next_header(TheoraPacketKind::IdentificationHeader)?)
                .map_err(invalid)?;
        let geometry = geometry(&ident)?;
        parse_comment_header(&next_header(TheoraPacketKind::CommentHeader)?).map_err(invalid)?;
        let configuration = next_header(TheoraPacketKind::SetupHeader)?;
        let mut data = Vec::new();
        let mut frames = Vec::new();
        let mut keyframe_index = 0;
        let mut origin = None;
        let mut begin = micros(offset)?;
        for packet in packets {
            let packet = packet?;
            let kind = classify_packet(&packet.data).map_err(invalid)?;
            if matches!(kind, TheoraPacketKind::ReservedHeader(_)) && frames.is_empty() {
                if packet.last_in_page() && packet.absgp_page() != 0 {
                    return Err(invalid("invalid Ogg Theora header granule"));
                }
                continue;
            }
            if kind != TheoraPacketKind::VideoData {
                return Err(invalid("unexpected Ogg Theora header after setup"));
            }
            let keyframe = packet.data.first().is_some_and(|flag| flag & 0x40 == 0);
            if keyframe {
                keyframe_index = frames.len() as u64;
            } else if frames.is_empty() {
                return Err(invalid("Ogg Theora stream has no initial keyframe"));
            }
            if packet.last_in_page() {
                let granule = packet.absgp_page();
                if granule > i64::MAX as u64 {
                    return Err(invalid("invalid Ogg Theora granule position"));
                }
                let granule = if ident.vrev == 0 {
                    granule_position_from_legacy(granule, ident.kfgshift).map_err(invalid)?
                } else {
                    granule
                };
                let marked =
                    frame_index_from_granule_position(granule, ident.kfgshift).map_err(invalid)?;
                let key = keyframe_index_from_granule_position(granule, ident.kfgshift)
                    .map_err(invalid)?;
                let base = *origin.get_or_insert(
                    marked
                        .checked_sub(frames.len() as u64)
                        .ok_or_else(|| invalid("Ogg Theora granule precedes its picture count"))?,
                );
                if marked.checked_sub(base) != Some(frames.len() as u64)
                    || key.checked_sub(base) != Some(keyframe_index)
                {
                    return Err(invalid(
                        "Ogg Theora granule disagrees with its picture sequence",
                    ));
                }
            }
            let end = micros(offset + elapsed_ns(&ident, frames.len() + 1))?;
            if end <= begin {
                return Err(invalid("Ogg video frames are shorter than a microsecond"));
            }
            let start = data.len();
            data.extend_from_slice(&packet.data);
            frames.push(FrameTime {
                bytes: start..data.len(),
                interval: begin..end,
                keyframe,
            });
            begin = end;
        }
        if frames.is_empty() || origin.is_none() {
            return Err(invalid("Ogg Theora has no timed pictures"));
        }
        Ok(Self {
            ident,
            geometry,
            configuration,
            data,
            frames,
            end_us: begin,
        })
    }

    fn sample_frames(&self, rate: u32) -> Result<usize, EngineError> {
        let frames = (self.frames.len() as u128 * u128::from(self.ident.frd) * u128::from(rate))
            .div_ceil(u128::from(self.ident.frn));
        usize::try_from(frames).map_err(|_| limited("Ogg chain duration exceeds limits"))
    }

    fn image(
        &self,
        decoder: &FrameDecoder,
        start: u64,
        duration: u64,
    ) -> Result<VideoImage, EngineError> {
        let refs = decoder.reference_store();
        let picture = Yuv {
            planes: [
                Plane {
                    bytes: &refs.previous_y,
                    stride: refs.dims_y.width as usize,
                },
                Plane {
                    bytes: &refs.previous_cb,
                    stride: refs.dims_c.width as usize,
                },
                Plane {
                    bytes: &refs.previous_cr,
                    stride: refs.dims_c.width as usize,
                },
            ],
            x: usize::from(self.ident.picx),
            y: usize::from(self.ident.picy),
            bottom_up: true,
        };
        match self.ident.pf {
            PixelFormat::Yuv420 => self.geometry.image::<2, 2>(picture, start, duration),
            PixelFormat::Yuv422 => self.geometry.image::<2, 1>(picture, start, duration),
            PixelFormat::Yuv444 => self.geometry.image::<1, 1>(picture, start, duration),
        }
    }
}

pub(in crate::epub) fn align_audio(
    bytes: &[u8],
    track: &mut opus::Track,
    named: Option<&NamedTracks>,
) -> Result<(), EngineError> {
    if track.link_lengths().len() < 2 || named.is_some_and(|tracks| !tracks.has_video()) {
        return Ok(());
    }
    let video = alignment_frames(bytes, Some(Codec::Opus), OUTPUT_SAMPLE_RATE_HZ, named)?;
    track.align_video_links(&video)
}

pub(in crate::epub) fn align_vorbis_audio(
    bytes: &[u8],
    track: &mut vorbis::Track,
    named: Option<&NamedTracks>,
) -> Result<(), EngineError> {
    if track.link_lengths().len() < 2 || named.is_some_and(|tracks| !tracks.has_video()) {
        return Ok(());
    }
    let video = alignment_frames(bytes, Some(Codec::Vorbis), track.spec().sample_rate, named)?;
    track.align_video_links(&video)
}

pub(in crate::epub) fn align_mixed_audio(
    bytes: &[u8],
    track: &mut ogg_mixed::Track,
    named: &NamedTracks,
) -> Result<(), EngineError> {
    if !named.has_video() {
        return Ok(());
    }
    let video = alignment_frames(bytes, None, track.spec().sample_rate, Some(named))?;
    track.align_video_links(&video)
}

fn alignment_frames(
    bytes: &[u8],
    codec: Option<Codec>,
    rate: u32,
    named: Option<&NamedTracks>,
) -> Result<Vec<usize>, EngineError> {
    let segments = ogg::groups(bytes, Codec::Theora, None, named)?;
    let soundtrack = match named {
        Some(tracks) => tracks.audio_codec(),
        None => chained_soundtrack(&segments)?,
    };
    if soundtrack != codec {
        return Err(invalid("Ogg video and soundtrack segments disagree"));
    }
    segments
        .into_iter()
        .map(|segment| Link::index(segment, 0)?.sample_frames(rate))
        .collect()
}

fn chained_soundtrack(segments: &[Segment<'_>]) -> Result<Option<Codec>, EngineError> {
    if segments.len() < 2 {
        return Ok(None);
    }
    if segments
        .iter()
        .all(|s| s.opus_tracks == 1 && s.vorbis_tracks == 0 && !s.has_other_audio)
    {
        return Ok(Some(Codec::Opus));
    }
    if segments
        .iter()
        .all(|s| s.vorbis_tracks == 1 && s.opus_tracks == 0 && !s.has_other_audio)
    {
        return Ok(Some(Codec::Vorbis));
    }
    if segments
        .iter()
        .any(|s| s.opus_tracks != 0 || s.vorbis_tracks != 0 || s.has_other_audio)
    {
        return Err(unsupported(
            "chained Ogg video requires one consistent Opus or Vorbis soundtrack in each segment",
        ));
    }
    Ok(None)
}

fn geometry(ident: &TheoraIdentHeader) -> Result<Geometry, EngineError> {
    let w = ident.coded_width();
    let h = ident.coded_height();
    if w > 4096 || h > 4096 || w.checked_mul(h).is_none_or(|area| area > MAX_PIXELS) {
        return Err(limited("Theora coded frame geometry exceeds limits"));
    }
    if ident.picw == 0 || ident.pich == 0 {
        return Err(invalid("empty Theora picture window"));
    }
    if matches!(ident.cs, ColorSpace::Reserved(_)) {
        return Err(unsupported("reserved Theora colour space"));
    }
    let aspect = if ident.parn == 0 || ident.pard == 0 {
        (1, 1)
    } else {
        (ident.parn, ident.pard)
    };
    Ok(Geometry {
        width: ident.picw,
        height: ident.pich,
        aspect,
        full_range: false,
        kr: 0.299,
        kb: 0.114,
        region: None,
    })
}

fn elapsed_ns(ident: &TheoraIdentHeader, frames: usize) -> u128 {
    frames as u128 * u128::from(ident.frd) * 1_000_000_000 / u128::from(ident.frn)
}

fn micros(nanos: u128) -> Result<u64, EngineError> {
    let micros = nanos / 1000;
    if micros > MAX_DURATION_US {
        return Err(limited("Ogg video duration exceeds its timing budget"));
    }
    Ok(micros as u64)
}
