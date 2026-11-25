//! Bounded WebVTT region definitions and cue-to-region association.
use std::{collections::HashMap, sync::Arc};

use super::{charge, limited, EngineError};

const MAX_REGION_BLOCKS: usize = 256;
const MAX_REGION_BYTES: usize = 4096;
const MAX_TRACK_REGION_BYTES: usize = 64 * 1024;
const MAX_REGION_ID_BYTES: usize = 1024;

pub(crate) struct Region {
    pub id: String,
    pub width_percent: f64,
    pub lines: u32,
    pub region_anchor_percent: (f64, f64),
    pub viewport_anchor_percent: (f64, f64),
    pub scroll_up: bool,
}

#[derive(Default)]
pub(super) struct Regions<'a> {
    entries: HashMap<&'a str, Arc<Region>>,
    blocks: usize,
    bytes: usize,
}

impl<'a> Regions<'a> {
    pub(super) fn block(
        &mut self,
        lines: impl Iterator<Item = &'a str>,
    ) -> Result<(), EngineError> {
        charge(
            &mut self.blocks,
            1,
            MAX_REGION_BLOCKS,
            "WebVTT exceeds 256 REGION blocks",
        )?;
        let mut block_bytes = 0;
        let mut id = None;
        let mut malformed = false;
        let mut region = Region {
            id: String::new(),
            width_percent: 100.0,
            lines: 3,
            region_anchor_percent: (0.0, 100.0),
            viewport_anchor_percent: (0.0, 100.0),
            scroll_up: false,
        };
        for line in lines {
            let bytes = line.len() + 1;
            charge(
                &mut block_bytes,
                bytes,
                MAX_REGION_BYTES,
                "WebVTT REGION block exceeds 4 KiB",
            )?;
            charge(
                &mut self.bytes,
                bytes,
                MAX_TRACK_REGION_BYTES,
                "WebVTT REGION blocks exceed 64 KiB",
            )?;
            if line.contains("-->") {
                malformed = true;
                continue;
            }
            for (name, value) in settings(line) {
                match name {
                    "id" => {
                        if value.len() > MAX_REGION_ID_BYTES {
                            return Err(limited(
                                "WebVTT REGION identifier exceeds its byte budget",
                            ));
                        }
                        id = Some(value);
                    }
                    "width" => {
                        if let Some(width) = percentage(value) {
                            region.width_percent = width;
                        }
                    }
                    "lines" if digits(value) => {
                        region.lines = value
                            .parse()
                            .map_err(|_| limited("WebVTT REGION line count exceeds 32 bits"))?;
                    }
                    "regionanchor" => {
                        if let Some(anchor) = anchor(value) {
                            region.region_anchor_percent = anchor;
                        }
                    }
                    "viewportanchor" => {
                        if let Some(anchor) = anchor(value) {
                            region.viewport_anchor_percent = anchor;
                        }
                    }
                    "scroll" if value == "up" => region.scroll_up = true,
                    _ => {}
                }
            }
        }
        if let Some(id) = id.filter(|_| !malformed) {
            region.id = id.to_owned();
            self.entries.insert(id, Arc::new(region));
        }
        Ok(())
    }

    pub(super) fn resolve(&self, source: &str) -> Option<Arc<Region>> {
        let mut region = None;
        let mut vertical = false;
        for (name, value) in settings(source) {
            match name {
                "region" => region = self.entries.get(value),
                "vertical" => {
                    vertical |= matches!(value, "rl" | "lr");
                    if vertical {
                        region = None;
                    }
                }
                "line" if explicitly_positioned(value) => region = None,
                "size" if percentage(value).is_some_and(|size| size != 100.0) => region = None,
                _ => {}
            }
        }
        region.cloned()
    }
}

fn settings(source: &str) -> impl Iterator<Item = (&str, &str)> {
    source.split_ascii_whitespace().filter_map(|token| {
        let (name, value) = token.split_once(':')?;
        (!name.is_empty() && !value.is_empty()).then_some((name, value))
    })
}

fn digits(source: &str) -> bool {
    !source.is_empty() && source.bytes().all(|byte| byte.is_ascii_digit())
}

fn decimal(source: &str) -> bool {
    source.split_once('.').map_or_else(
        || digits(source),
        |(whole, fraction)| digits(whole) && digits(fraction),
    )
}

fn percentage(source: &str) -> Option<f64> {
    let number = source.strip_suffix('%').filter(|number| decimal(number))?;
    number
        .parse::<f64>()
        .ok()
        .filter(|number| (0.0..=100.0).contains(number))
}

fn anchor(source: &str) -> Option<(f64, f64)> {
    let (x, y) = source.split_once(',')?;
    Some((percentage(x)?, percentage(y)?))
}

fn explicitly_positioned(source: &str) -> bool {
    let (line, align) = source
        .split_once(',')
        .map_or((source, None), |(line, align)| (line, Some(align)));
    if align.is_some_and(|value| !matches!(value, "start" | "center" | "end")) {
        return false;
    }
    if line.ends_with('%') {
        percentage(line).is_some()
    } else {
        decimal(line.strip_prefix('-').unwrap_or(line))
            && line.parse::<f64>().is_ok_and(f64::is_finite)
    }
}
