//! The shared rational-media clock for TTML caption and region timing.
use roxmltree::Node;

use super::{invalid, limited, media::integer, ttml_styles::PARAM, unsupported};
use crate::EngineError;

pub(super) const MICROS: u64 = 1_000_000;
pub(super) const MAX_TIME: u64 = (1 << 53) - 1;
// Nine decimal places plus the millisecond conversion fit into this scale.
const DECIMALS: u128 = 1_000_000_000_000;

pub(super) type Instant = u128;

#[derive(Clone, Copy)]
pub(super) struct Clock {
    units: u128,
    frame_rate: u64,
    frame_num: u128,
    frame_den: u128,
    subframes: u128,
    tick_num: u128,
    tick_den: u128,
}

impl Clock {
    pub(super) fn new(root: Node<'_, '_>) -> Result<Self, EngineError> {
        if root
            .attribute((PARAM, "timeBase"))
            .is_some_and(|base| base != "media")
        {
            return Err(unsupported("TTML requires the media time base"));
        }
        let rate = |name: &str, fallback: u64| {
            root.attribute((PARAM, name))
                .map_or(Ok(fallback), positive_rate)
        };
        let frame_rate = rate("frameRate", 30)?;
        let subframes = u128::from(rate("subFrameRate", 1)?);
        let (num, den) = if let Some(value) = root.attribute((PARAM, "frameRateMultiplier")) {
            let mut parts = value.split_ascii_whitespace();
            let (Some(num), Some(den), None) = (parts.next(), parts.next(), parts.next()) else {
                return Err(invalid("invalid TTML frame rate multiplier"));
            };
            (positive_rate(num)?, positive_rate(den)?)
        } else {
            (1, 1)
        };
        let frame_num = u128::from(frame_rate) * u128::from(num);
        let frame_den = u128::from(den);
        let frame_ticks = frame_num * subframes;
        let (tick_num, tick_den) = if let Some(value) = root.attribute((PARAM, "tickRate")) {
            (u128::from(positive_rate(value)?), 1)
        } else if root.has_attribute((PARAM, "frameRate")) {
            (frame_ticks, frame_den)
        } else {
            (1, 1)
        };
        let units = DECIMALS * (frame_ticks / gcd(frame_ticks, tick_num)) * tick_num;
        if units > 10u128.pow(24) {
            return Err(limited(
                "TTML timing parameters exceed their precision budget",
            ));
        }
        Ok(Self {
            units,
            frame_rate,
            frame_num,
            frame_den,
            subframes,
            tick_num,
            tick_den,
        })
    }

    fn parse(self, value: &str) -> Result<Instant, EngineError> {
        let value = value.trim();
        let malformed = || invalid("invalid TTML time expression");
        if value.contains(':') {
            let mut fields = value.split(':');
            let h = fields
                .next()
                .filter(|s| s.len() >= 2)
                .and_then(integer)
                .ok_or_else(malformed)?;
            let m = fields
                .next()
                .filter(|s| s.len() == 2)
                .and_then(integer)
                .filter(|&n| n < 60)
                .ok_or_else(malformed)?;
            let s = fields.next().ok_or_else(malformed)?;
            if s.split('.').next().is_none_or(|s| s.len() != 2) {
                return Err(malformed());
            }
            let (num, den) = decimal(s).ok_or_else(malformed)?;
            if num >= 60 * den {
                return Err(malformed());
            }
            let seconds = (u128::from(h) * 3600 + u128::from(m) * 60) * den + num;
            let time = self.seconds(seconds, den)?;
            let Some(frame) = fields.next() else {
                return Ok(time);
            };
            if s.contains('.') || fields.next().is_some() {
                return Err(malformed());
            }
            let (frame, subframe) = if let Some((frame, subframe)) = frame.split_once('.') {
                (frame, u128::from(integer(subframe).ok_or_else(malformed)?))
            } else {
                (frame, 0)
            };
            let frame = integer(frame)
                .filter(|&n| frame.len() >= 2 && n < self.frame_rate)
                .ok_or_else(malformed)?;
            if subframe >= self.subframes {
                return Err(malformed());
            }
            let offset = self.seconds(
                (u128::from(frame) * self.subframes + subframe) * self.frame_den,
                self.frame_num * self.subframes,
            )?;
            return self.add(time, offset);
        }
        let (number, multiplier, divisor) = [
            ("ms", 1, 1000),
            ("h", 3600, 1),
            ("m", 60, 1),
            ("s", 1, 1),
            ("f", self.frame_den, self.frame_num),
            ("t", self.tick_den, self.tick_num),
        ]
        .into_iter()
        .find_map(|(unit, mul, div)| value.strip_suffix(unit).map(|number| (number, mul, div)))
        .ok_or_else(malformed)?;
        let (num, den) = decimal(number).ok_or_else(malformed)?;
        self.seconds(
            num.checked_mul(multiplier).ok_or_else(malformed)?,
            den * divisor,
        )
    }

    fn seconds(self, num: u128, den: u128) -> Result<Instant, EngineError> {
        // The document-wide clock is a multiple of every supported time unit.
        let time = (self.units / den)
            .checked_mul(num)
            .ok_or_else(|| invalid("TTML time is out of range"))?;
        self.add(time, 0)
    }

    pub(super) fn add(self, a: Instant, b: Instant) -> Result<Instant, EngineError> {
        let time = a
            .checked_add(b)
            .ok_or_else(|| invalid("TTML time is out of range"))?;
        if time > u128::from(MAX_TIME) * (self.units / u128::from(MICROS)) {
            return Err(invalid("TTML time is out of range"));
        }
        Ok(time)
    }

    #[cfg(feature = "python")]
    pub(super) fn micros(self, time: Instant) -> u64 {
        let units = self.units / u128::from(MICROS);
        ((time + units / 2) / units) as u64
    }
}

fn positive_rate(value: &str) -> Result<u64, EngineError> {
    integer(value.trim())
        .filter(|rate| (1..=1_000_000).contains(rate))
        .ok_or_else(|| invalid("invalid or excessive TTML timing rate"))
}

fn decimal(value: &str) -> Option<(u128, u128)> {
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction))
            if !fraction.is_empty() && fraction.bytes().all(|ch| ch.is_ascii_digit()) =>
        {
            (whole, fraction.trim_end_matches('0'))
        }
        Some(_) => return None,
        None => (value, ""),
    };
    if fraction.len() > 9 {
        return None;
    }
    let scale = 10u128.pow(fraction.len() as u32);
    let whole = u128::from(integer(whole)?);
    let sub = if fraction.is_empty() {
        0
    } else {
        u128::from(integer(fraction)?)
    };
    Some((whole * scale + sub, scale))
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[derive(Clone, Copy)]
pub(super) struct Timing {
    pub(super) begin: Instant,
    pub(super) end: Option<Instant>,
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(super) explicit_end: bool,
}

impl Timing {
    pub(super) fn new(
        node: Node<'_, '_>,
        base: Instant,
        cap: Option<Instant>,
        clock: Clock,
    ) -> Result<Self, EngineError> {
        Self::from_attributes(
            [
                node.attribute("begin"),
                node.attribute("end"),
                node.attribute("dur"),
            ],
            base,
            cap,
            clock,
        )
    }

    fn from_attributes(
        [begin, end, duration]: [Option<&str>; 3],
        base: Instant,
        cap: Option<Instant>,
        clock: Clock,
    ) -> Result<Self, EngineError> {
        let begin = clock.add(
            base,
            begin
                .map(|value| clock.parse(value))
                .transpose()?
                .unwrap_or(0),
        )?;
        let end = end
            .map(|end| clock.parse(end).and_then(|end| clock.add(base, end)))
            .transpose()?;
        let duration = duration
            .map(|dur| clock.parse(dur).and_then(|dur| clock.add(begin, dur)))
            .transpose()?;
        Ok(Self {
            begin,
            end: cap.into_iter().chain(end).chain(duration).min(),
            explicit_end: end.is_some() || duration.is_some(),
        })
    }

    pub(super) fn empty(self) -> bool {
        self.end.is_some_and(|end| end <= self.begin)
    }

    #[cfg(feature = "python")]
    pub(super) fn finish(self, implied: Option<Instant>) -> Option<Instant> {
        if self.explicit_end {
            self.end
        } else {
            implied
        }
    }
}
