//! Bound percentage length ratios for comparisons and absolute values.
use super::{Comparison, Expression, Interval, Issue};

// Each pair is an affine length: points plus a percentage of a nonnegative
// containing extent. Separate lines bound the whole length, not each value
// at one particular viewport.
#[derive(Clone, Copy)]
struct Envelope {
    lower: [f64; 2],
    upper: [f64; 2],
}

impl Envelope {
    fn from_expression(expression: &Expression) -> Option<Self> {
        Some(match expression {
            Expression::Linear(points, percent) => {
                let coefficients = [
                    f64::from(points.unwrap_or_default()),
                    f64::from(percent.unwrap_or_default()) / 100.0,
                ];
                if !coefficients.into_iter().all(f64::is_finite) {
                    return None;
                }
                Self {
                    lower: coefficients,
                    upper: coefficients,
                }
            }
            Expression::Sum(left, right) => {
                let left = Self::from_expression(left)?;
                let right = Self::from_expression(right)?;
                Self {
                    lower: std::array::from_fn(|i| left.lower[i] + right.lower[i]),
                    upper: std::array::from_fn(|i| left.upper[i] + right.upper[i]),
                }
            }
            Expression::Scale(value, factor) if factor.is_finite() => {
                Self::from_expression(value)?.scale(f64::from(*factor))
            }
            Expression::Abs(value) => {
                let value = Self::from_expression(value)?;
                if nonnegative(value.lower) {
                    value
                } else if nonpositive(value.upper) {
                    value.scale(-1.0)
                } else {
                    Self {
                        lower: [0.0; 2],
                        upper: value.magnitude(),
                    }
                }
            }
            Expression::Compare(comparison, values) => Self::compare(*comparison, values)?,
            Expression::Hypot(values) => Self::hypot(values)?,
            _ => return None,
        })
    }

    fn scale(self, factor: f64) -> Self {
        let (lower, upper) = if factor < 0.0 {
            (self.upper, self.lower)
        } else {
            (self.lower, self.upper)
        };
        Self {
            lower: lower.map(|value| value * factor),
            upper: upper.map(|value| value * factor),
        }
    }

    fn magnitude(self) -> [f64; 2] {
        std::array::from_fn(|i| self.lower[i].abs().max(self.upper[i].abs()))
    }

    fn hypot(values: &[Expression]) -> Option<Self> {
        let mut largest_lower = [0.0f64; 2];
        let mut upper = [0.0f64; 2];
        for value in values {
            let value = Self::from_expression(value)?;
            let lower = if nonnegative(value.lower) {
                value.lower
            } else if nonpositive(value.upper) {
                value.upper.map(|coefficient| -coefficient)
            } else {
                [0.0; 2]
            };
            let magnitude = value.magnitude();
            for i in 0..2 {
                largest_lower[i] = largest_lower[i].max(lower[i]);
                upper[i] = upper[i].hypot(magnitude[i]);
            }
        }
        // The Euclidean norm bounds each single magnitude from below.
        // The triangle inequality applies separately to the constant and
        // percentage coefficient vectors for the upper envelope.
        Some(Self {
            lower: largest_lower.map(|coefficient| coefficient * 0.5),
            upper,
        })
    }

    fn compare(comparison: Comparison, values: &[Expression]) -> Option<Self> {
        let mut values = values.iter().map(Self::from_expression);
        let mut result = values.next()??;
        let mut best_upper = result.upper;
        let mut best_lower = result.lower;
        let mut smallest_upper = result.upper;
        let mut every_lower_nonnegative = nonnegative(result.lower);
        let mut every_upper_nonpositive = nonpositive(result.upper);
        for value in values {
            let value = value?;
            result.lower = std::array::from_fn(|i| result.lower[i].min(value.lower[i]));
            result.upper = std::array::from_fn(|i| result.upper[i].max(value.upper[i]));
            best_upper = [best_upper, value.upper]
                .into_iter()
                .min_by(|a, b| a[1].total_cmp(&b[1]).then_with(|| a[0].total_cmp(&b[0])))?;
            best_lower = std::array::from_fn(|i| best_lower[i].max(value.lower[i]));
            smallest_upper = std::array::from_fn(|i| smallest_upper[i].min(value.upper[i]));
            every_lower_nonnegative &= nonnegative(value.lower);
            every_upper_nonpositive &= nonpositive(value.upper);
        }
        match comparison {
            // The minimum is at most any one operand. Prefer an envelope
            // with the smallest growth, which can prove a constant cap.
            Comparison::Min if every_upper_nonpositive => {
                // The same argument after a sign change bounds a negative minimum.
                result.upper = smallest_upper.map(|value| value * 0.5);
            }
            Comparison::Min => result.upper = best_upper,
            // With nonnegative coefficients, the maximum is at least both
            // the largest constant and the largest slope times the basis.
            // Their average is a valid affine lower bound.
            Comparison::Max if every_lower_nonnegative => {
                result.lower = best_lower.map(|value| value * 0.5);
            }
            Comparison::Max => {}
        }
        Some(result)
    }
}

fn nonnegative(coefficients: [f64; 2]) -> bool {
    coefficients.into_iter().all(|value| value >= 0.0)
}

fn nonpositive(coefficients: [f64; 2]) -> bool {
    coefficients.into_iter().all(|value| value <= 0.0)
}

pub(super) fn bounds(numerator: &Expression, denominator: &Expression) -> Result<Interval, Issue> {
    let mut numerator = Envelope::from_expression(numerator).ok_or(Issue::Unsupported)?;
    let mut denominator = Envelope::from_expression(denominator).ok_or(Issue::Unsupported)?;
    if nonpositive(denominator.upper) && denominator.upper != [0.0; 2] {
        denominator = denominator.scale(-1.0);
        numerator = numerator.scale(-1.0);
    }
    if !nonnegative(denominator.lower) || denominator.lower == [0.0; 2] {
        return Err(Issue::Unsupported);
    }

    // A positive denominator's affine lower line can absorb a numerator
    // term only when it grows in the corresponding coefficient. Shared
    // zero terms vanish, including for a pure-percentage denominator at 0.
    let mut magnitude = 0.0f64;
    for (top, bottom) in numerator.magnitude().into_iter().zip(denominator.lower) {
        if bottom > 0.0 {
            magnitude = magnitude.max(top / bottom);
        } else if top != 0.0 {
            return Err(Issue::Unsupported);
        }
    }
    if !magnitude.is_finite() {
        return Err(Issue::Limit);
    }
    let minimum = |top: [f64; 2]| {
        top.into_iter()
            .zip(denominator.upper)
            .filter(|(_, bottom)| *bottom > 0.0)
            .map(|(top, bottom)| top / bottom)
            .fold(magnitude, f64::min)
    };
    Ok(if nonnegative(numerator.lower) {
        Interval {
            low: minimum(numerator.lower) as f32,
            high: magnitude as f32,
        }
    } else if nonpositive(numerator.upper) {
        Interval {
            low: -magnitude as f32,
            high: -minimum(numerator.upper.map(|value| -value)) as f32,
        }
    } else {
        Interval::signed(magnitude as f32)
    })
}

fn wide_value(expression: &Expression, basis: f64) -> Option<f64> {
    Some(match expression {
        Expression::Linear(points, percent) => {
            f64::from(points.unwrap_or_default()) + f64::from(percent.unwrap_or_default()) * basis
        }
        Expression::Sum(left, right) => wide_value(left, basis)? + wide_value(right, basis)?,
        Expression::Scale(value, factor) => wide_value(value, basis)? * f64::from(*factor),
        Expression::Abs(value) => wide_value(value, basis)?.abs(),
        Expression::Hypot(values) => values.iter().try_fold(0.0f64, |norm, value| {
            Some(norm.hypot(wide_value(value, basis)?))
        })?,
        Expression::Compare(comparison, values) => {
            let mut values = values.iter().map(|value| wide_value(value, basis));
            let first = values.next()??;
            values.try_fold(first, |acc, value| {
                Some(match comparison {
                    Comparison::Min => acc.min(value?),
                    Comparison::Max => acc.max(value?),
                })
            })?
        }
        _ => return None,
    })
}

pub(super) fn evaluate(numerator: &Expression, denominator: &Expression, basis: f32) -> f32 {
    let basis = f64::from(basis.max(0.0)) / 100.0;
    let numerator = wide_value(numerator, basis).unwrap_or(f64::NAN);
    let denominator = wide_value(denominator, basis).unwrap_or(f64::NAN);
    (numerator / denominator) as f32
}
