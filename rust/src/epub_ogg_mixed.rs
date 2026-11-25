//! Keep a selected Ogg soundtrack on one PCM clock across codec changes.
use std::ops::Range;

use hound::WavSpec;

use super::{invalid, limited, opus, unsupported, vorbis, EngineError, MAX_CLIP_SAMPLES};
use crate::epub::ogg::{self, Codec, SelectedTracks, MAX_PACKETS};

pub(in crate::epub) struct Track {
    links: Vec<Link>,
    spec: WavSpec,
    speaker_mask: Option<u32>,
    frames: usize,
    active: Option<usize>,
}

struct Link {
    decoder: Decoder,
    timeline: Range<usize>,
}

enum Decoder {
    Opus(Box<opus::Track>),
    Vorbis(Box<vorbis::Track>),
}

impl Decoder {
    fn geometry(&self) -> (WavSpec, Option<u32>) {
        match self {
            Self::Opus(track) => (track.spec(), track.speaker_mask()),
            Self::Vorbis(track) => (track.spec(), track.speaker_mask()),
        }
    }

    fn frames(&self) -> usize {
        match self {
            Self::Opus(track) => track.frames(),
            Self::Vorbis(track) => track.frames(),
        }
    }

    fn release_playback(&mut self) {
        match self {
            Self::Opus(track) => track.release_playback(),
            Self::Vorbis(track) => track.release_playback(),
        }
    }

    fn samples(
        &mut self,
        start: usize,
        stop: usize,
        work: &mut usize,
    ) -> Result<Vec<i16>, EngineError> {
        match self {
            Self::Opus(track) => track.samples_with_work(start, stop, work),
            Self::Vorbis(track) => track.samples_with_work(start, stop, work),
        }
    }
}

impl Track {
    pub(in crate::epub) fn open(
        bytes: &[u8],
        selection: &SelectedTracks,
    ) -> Result<Self, EngineError> {
        let mut opus_packets = MAX_PACKETS;
        let mut vorbis_codebooks = 0;
        let segments = ogg::selected_audio_groups(bytes, selection)?;
        let mut links = Vec::with_capacity(segments.len());
        let mut geometry = None;
        for segment in segments {
            let decoder = match segment.codec {
                Codec::Opus => Decoder::Opus(Box::new(opus::Track::open_ogg_link(
                    segment.pages,
                    &mut opus_packets,
                )?)),
                Codec::Vorbis => Decoder::Vorbis(Box::new(vorbis::Track::open_ogg_link(
                    segment.pages,
                    &mut vorbis_codebooks,
                )?)),
                Codec::Theora => return Err(invalid("Ogg soundtrack selected a video stream")),
            };
            let link_format = decoder.geometry();
            if *geometry.get_or_insert(link_format) != link_format {
                return Err(unsupported(
                    "mixed Opus and Vorbis soundtrack requires one 48 kHz channel layout",
                ));
            }
            links.push(Link {
                decoder,
                timeline: 0..0,
            });
        }
        let (spec, speaker_mask) = geometry.ok_or_else(|| invalid("empty Ogg soundtrack"))?;
        let mut track = Self {
            links,
            spec,
            speaker_mask,
            frames: 0,
            active: None,
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

    pub(in crate::epub) fn link_lengths(&self) -> impl Iterator<Item = usize> + '_ {
        self.links.iter().map(|link| link.decoder.frames())
    }

    pub(in crate::epub) fn align_video_links(
        &mut self,
        pictures: &[usize],
    ) -> Result<(), EngineError> {
        self.place_links(Some(pictures))
    }

    fn place_links(&mut self, pictures: Option<&[usize]>) -> Result<(), EngineError> {
        if pictures.is_some_and(|pictures| pictures.len() != self.links.len()) {
            return Err(invalid("Ogg video and soundtrack segments disagree"));
        }
        let last = self.links.len() - 1;
        let mut cursor = 0usize;
        for (index, link) in self.links.iter_mut().enumerate() {
            let video_frames = pictures
                .filter(|_| index < last)
                .map_or(0, |frames| frames[index]);
            let begin = cursor;
            cursor = cursor
                .checked_add(link.decoder.frames().max(video_frames))
                .ok_or_else(|| limited("Ogg chain duration exceeds its frame budget"))?;
            link.timeline = begin..cursor;
        }
        self.frames = cursor;
        Ok(())
    }

    pub(super) fn samples(&mut self, start: usize, stop: usize) -> Result<Vec<i16>, EngineError> {
        if start >= stop || stop > self.frames {
            return Err(invalid("mixed Ogg clip lies outside its presentation"));
        }
        let channels = usize::from(self.spec.channels);
        if stop - start > MAX_CLIP_SAMPLES / channels {
            return Err(limited("media-overlay audio clip exceeds 16 MiB of PCM"));
        }
        let mut output = vec![0; (stop - start) * channels];
        let mut work = 0;
        let first = self
            .links
            .partition_point(|link| link.timeline.end <= start);
        let end = self
            .links
            .partition_point(|link| link.timeline.start < stop);
        for index in first..end {
            let offset = self.links[index].timeline.start;
            let begin = start.saturating_sub(offset);
            let audible = (stop - offset).min(self.links[index].decoder.frames());
            if begin >= audible {
                continue;
            }
            if let Some(previous) = self
                .active
                .replace(index)
                .filter(|&previous| previous != index)
            {
                self.links[previous].decoder.release_playback();
            }
            let samples = self.links[index]
                .decoder
                .samples(begin, audible, &mut work)?;
            let output_start = (offset + begin - start) * channels;
            let output_count = (audible - begin) * channels;
            if samples.len() != output_count {
                return Err(invalid("mixed Ogg clip returned unexpected PCM geometry"));
            }
            output[output_start..output_start + output_count].copy_from_slice(&samples);
        }
        Ok(output)
    }
}
