//! Finite CSS exponential functions and conservative bounds for deferred scalars.

#[derive(Clone, Copy)]
pub(super) struct Interval {
    pub low: f32,
    pub high: f32,
}

impl Interval {
    pub(super) fn point(value: f32) -> Self {
        Self {
            low: value,
            high: value,
        }
    }

    pub(super) fn signed(bound: f32) -> Self {
        Self {
            low: -bound,
            high: bound,
        }
    }

    pub(super) fn absolute(self) -> Self {
        let a = self.low.abs();
        let b = self.high.abs();
        Self {
            low: if self.low <= 0.0 && self.high >= 0.0 {
                0.0
            } else {
                a.min(b)
            },
            high: a.max(b),
        }
    }

    fn corners(self, other: Self, operation: impl Fn(f32, f32) -> f32) -> Self {
        let values = [
            operation(self.low, other.low),
            operation(self.low, other.high),
            operation(self.high, other.low),
            operation(self.high, other.high),
        ];
        Self {
            low: values.into_iter().fold(f32::INFINITY, f32::min),
            high: values.into_iter().fold(f32::NEG_INFINITY, f32::max),
        }
    }

    pub(super) fn product(self, other: Self) -> Self {
        self.corners(other, |one, other| one * other)
    }
}

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Power,
    Root,
    Exp,
    Ln,
    Log,
}

impl Operation {
    pub(super) fn evaluate(self, value: f32, parameter: f32) -> f32 {
        if value.is_nan() || parameter.is_nan() {
            return f32::NAN;
        }
        let value = f64::from(value);
        let parameter = f64::from(parameter);
        (match self {
            Self::Power => value.powf(parameter),
            Self::Root => value.sqrt(),
            Self::Exp => value.exp(),
            Self::Ln => value.ln(),
            Self::Log if parameter <= 0.0 || parameter == 1.0 => f64::NAN,
            Self::Log => value.log(parameter),
        }) as f32
    }

    pub(super) fn bounds(self, value: Interval, parameter: Interval) -> Option<Interval> {
        let eval = |value, parameter| self.evaluate(value, parameter);
        Some(match self {
            Self::Root => Interval {
                low: eval(value.low.max(0.0), 0.0),
                high: eval(value.high.max(0.0), 0.0),
            },
            Self::Exp => Interval {
                low: eval(value.low, 0.0),
                high: eval(value.high, 0.0),
            },
            Self::Power => {
                let magnitude = value.absolute();
                if magnitude.low == 0.0 && parameter.low < 0.0 {
                    return None;
                }
                let magnitude = magnitude.corners(parameter, eval);
                if value.low < 0.0 {
                    Interval::signed(magnitude.high)
                } else {
                    magnitude
                }
            }
            Self::Ln if value.low > 0.0 => Interval {
                low: eval(value.low, 0.0),
                high: eval(value.high, 0.0),
            },
            Self::Log
                if value.low > 0.0
                    && parameter.low > 0.0
                    && (parameter.high < 1.0 || parameter.low > 1.0) =>
            {
                value.corners(parameter, eval)
            }
            _ => return None,
        })
    }
}

pub(super) fn hypot(values: impl IntoIterator<Item = f32>) -> f32 {
    values.into_iter().fold(0.0f64, |norm, value| {
        if norm.is_nan() || value.is_nan() {
            f64::NAN
        } else {
            norm.hypot(f64::from(value))
        }
    }) as f32
}
