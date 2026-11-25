//! Bounded CSS trigonometry with degrees as the canonical angle unit.

#[derive(Clone, Copy)]
pub(super) enum Input {
    Degrees,
    Radians,
}

#[derive(Clone, Copy)]
pub(super) enum Trig {
    Sin(Input),
    Cos(Input),
    Tan(Input),
    Asin,
    Acos,
    Atan,
}

impl Input {
    fn radians(self, value: f32) -> f64 {
        match self {
            Self::Degrees => f64::from(value).to_radians(),
            Self::Radians => f64::from(value),
        }
    }

    fn quarter_turn(self) -> f32 {
        match self {
            Self::Degrees => 90.0,
            Self::Radians => std::f32::consts::FRAC_PI_2,
        }
    }

    fn cardinal(self, value: f32) -> Option<i32> {
        let quarter = self.quarter_turn();
        (value % quarter == 0.0).then(|| ((value / quarter).rem_euclid(4.0)) as i32)
    }
}

impl Trig {
    pub(super) fn evaluate(self, value: f32) -> f32 {
        match self {
            Self::Sin(input) => match input.cardinal(value) {
                Some(0 | 2) => 0.0f32.copysign(value),
                Some(1) => 1.0,
                Some(3) => -1.0,
                _ => input.radians(value).sin() as f32,
            },
            Self::Cos(input) => match input.cardinal(value) {
                Some(0) => 1.0,
                Some(2) => -1.0,
                Some(1 | 3) => 0.0,
                _ => input.radians(value).cos() as f32,
            },
            Self::Tan(input) => match input.cardinal(value) {
                Some(0 | 2) => 0.0f32.copysign(value),
                Some(1) => f32::INFINITY,
                Some(3) => f32::NEG_INFINITY,
                _ => input.radians(value).tan() as f32,
            },
            Self::Asin => f64::from(value).asin().to_degrees() as f32,
            Self::Acos => f64::from(value).acos().to_degrees() as f32,
            Self::Atan => f64::from(value).atan().to_degrees() as f32,
        }
    }

    pub(super) fn bound(self, maximum: f32) -> Option<f32> {
        Some(match self {
            Self::Sin(_) | Self::Cos(_) => 1.0,
            Self::Asin => 90.0,
            Self::Acos => 180.0,
            Self::Atan => self.evaluate(maximum),
            Self::Tan(input) if maximum < input.quarter_turn() => self.evaluate(maximum),
            Self::Tan(_) => return None,
        })
    }
}

pub(super) fn degrees(value: f32, unit: &str) -> Option<f32> {
    let value = f64::from(value);
    Some(match unit.to_ascii_lowercase().as_str() {
        "deg" => value,
        "grad" => value * 0.9,
        "rad" => value.to_degrees(),
        "turn" => value * 360.0,
        _ => return None,
    } as f32)
}

pub(super) fn atan2(y: f32, x: f32) -> f32 {
    f64::from(y).atan2(f64::from(x)).to_degrees() as f32
}
