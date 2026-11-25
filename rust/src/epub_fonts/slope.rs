//! Resolve an oblique interval before the shaper chooses weight and glyph fallback.
use std::cmp::Ordering;

use parley::FontStyle;

const PREFERENCE_ANGLE: f32 = 11.0;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::epub) enum Request {
    Normal,
    Italic,
    Oblique(u32),
}

impl Request {
    pub(in crate::epub) fn new(italic: Option<bool>, oblique: Option<f32>) -> Self {
        if let Some(angle) = oblique {
            Self::Oblique(angle.to_bits())
        } else if italic == Some(true) {
            Self::Italic
        } else {
            Self::Normal
        }
    }

    pub(in crate::epub) fn style(self) -> FontStyle {
        match self {
            Self::Normal => FontStyle::Normal,
            Self::Italic => FontStyle::Italic,
            Self::Oblique(angle) => FontStyle::Oblique(Some(f32::from_bits(angle))),
        }
    }

    pub(in crate::epub) fn angle(self, [low, high]: [f32; 2]) -> f32 {
        let requested = match self {
            Self::Normal => 0.0,
            Self::Italic => PREFERENCE_ANGLE,
            Self::Oblique(angle) => f32::from_bits(angle),
        };
        requested.clamp(low, high)
    }

    pub(in crate::epub) fn rank(self, style: FontStyle, oblique: Option<[f32; 2]>) -> Rank {
        let selected = oblique.map(|range| self.angle(range));
        let italic = style == FontStyle::Italic;
        match self {
            Self::Normal => match selected {
                Some(angle) if angle >= 0.0 => Rank(0, angle),
                Some(angle) => Rank(2, -angle),
                None if italic => Rank(1, 0.0),
                None => Rank(0, 0.0),
            },
            Self::Italic => match selected {
                Some(angle) if angle >= PREFERENCE_ANGLE => Rank(1, angle - PREFERENCE_ANGLE),
                Some(angle) if angle > 0.0 => Rank(2, PREFERENCE_ANGLE - angle),
                Some(angle) => Rank(4, -angle),
                None if italic => Rank(0, 0.0),
                None => Rank(3, 0.0),
            },
            Self::Oblique(requested) => {
                let requested = f32::from_bits(requested);
                let direction = if requested < 0.0 { -1.0 } else { 1.0 };
                match selected {
                    Some(angle) if requested == angle => Rank(0, 0.0),
                    None if !italic && requested == 0.0 => Rank(0, 0.0),
                    Some(angle) if angle * direction > 0.0 => {
                        let steeper = angle.abs() > requested.abs();
                        let preferred = steeper == (requested.abs() >= PREFERENCE_ANGLE);
                        Rank(if preferred { 1 } else { 2 }, (angle - requested).abs())
                    }
                    // Upright faces provide the native shaper's synthetic match.
                    None if !italic => Rank(3, 0.0),
                    None => Rank(4, 0.0),
                    Some(angle) => Rank(5, angle.abs()),
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::epub) struct Rank(u8, f32);

impl Rank {
    pub(in crate::epub) fn cmp(self, other: Self) -> Ordering {
        self.0
            .cmp(&other.0)
            .then_with(|| self.1.total_cmp(&other.1))
    }
}
