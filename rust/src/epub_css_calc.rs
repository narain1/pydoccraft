//! Resolve bounded CSS calculations to the units accepted by the EPUB layout tree.
use raffia::{
    ast::{CalcOperatorKind as Op, ComponentValue, Declaration, DimensionKind, FunctionName},
    Parser, Syntax,
};

use super::{ident, points};
use crate::{
    epub::{limited, unsupported},
    EngineError,
};

const MAX_EXPRESSION_BYTES: usize = 512;
const MAX_RESULT: f32 = 25_000.0;

enum Value {
    Number(f32),
    Length {
        points: Option<f32>,
        percent: Option<f32>,
    },
}

impl Value {
    fn scale(self, factor: f32) -> Self {
        match self {
            Self::Number(value) => Self::Number(value * factor),
            Self::Length { points, percent } => Self::Length {
                points: points.map(|value| value * factor),
                percent: percent.map(|value| value * factor),
            },
        }
    }

    fn add(self, other: Self) -> Result<Self, Issue> {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => Ok(Self::Number(a + b)),
            (
                Self::Length { points, percent },
                Self::Length {
                    points: other_points,
                    percent: other_percent,
                },
            ) => {
                let add = |a: Option<f32>, b: Option<f32>| {
                    a.or(b).map(|_| a.unwrap_or(0.0) + b.unwrap_or(0.0))
                };
                Ok(Self::Length {
                    points: add(points, other_points),
                    percent: add(percent, other_percent),
                })
            }
            _ => Err(Issue::Invalid),
        }
    }
}

pub(in crate::epub) struct MixedLength {
    points: f32,
    percent: f32,
    signed: bool,
}

impl MixedLength {
    pub(in crate::epub) fn at(&self, basis: f32) -> f32 {
        let length = self.points + basis * (self.percent / 100.0);
        if self.signed {
            length
        } else {
            length.max(0.0)
        }
    }

    pub(super) fn has_fixed_part(&self) -> bool {
        self.points != 0.0
    }

    pub(super) fn computed_css(&self) -> String {
        format!("calc({}pt + {}%)", self.points, self.percent)
    }
}

pub(super) enum Resolved {
    Simple(String),
    Mixed(MixedLength),
}

enum Issue {
    Invalid,
    Unsupported,
    Limit,
}

pub(super) fn is_calc(value: &str) -> bool {
    value
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("calc("))
}

fn read(source: &str, font: f32, viewport: [f32; 2]) -> Result<Value, Issue> {
    if source.len() > MAX_EXPRESSION_BYTES {
        return Err(Issue::Limit);
    }
    let declaration = format!("value:{source}");
    let parsed = Parser::new(&declaration, Syntax::Css)
        .parse::<Declaration>()
        .map_err(|_| Issue::Invalid)?;
    let [expr] = parsed.value.as_slice() else {
        return Err(Issue::Invalid);
    };
    if parsed.important.is_some() || parsed.span.end != declaration.len() {
        return Err(Issue::Invalid);
    }
    evaluate(expr, &declaration, font, viewport)
}

fn evaluate(
    expr: &ComponentValue<'_>,
    source: &str,
    font: f32,
    viewport: [f32; 2],
) -> Result<Value, Issue> {
    let value = match expr {
        ComponentValue::Number(number) => Value::Number(number.value),
        ComponentValue::Percentage(percent) => Value::Length {
            points: None,
            percent: Some(percent.value.value),
        },
        ComponentValue::Dimension(dimension) if dimension.kind == DimensionKind::Length => {
            let unit = dimension.unit.name.to_ascii_lowercase();
            let raw = format!("{}{unit}", dimension.value.raw);
            let resolved = points(&raw, font, viewport).ok_or(Issue::Unsupported)?;
            Value::Length {
                points: Some(resolved),
                percent: None,
            }
        }
        ComponentValue::Dimension(_) => return Err(Issue::Invalid),
        ComponentValue::Function(function) => {
            if !matches!(&function.name, FunctionName::Ident(name)
                if ident(name).is_some_and(|name| name.eq_ignore_ascii_case("calc")))
            {
                return Err(Issue::Unsupported);
            }
            let [expr] = function.args.as_slice() else {
                return Err(Issue::Invalid);
            };
            return evaluate(expr, source, font, viewport);
        }
        ComponentValue::Calc(expr) => {
            if matches!(expr.op.kind, Op::Plus | Op::Minus) {
                let bytes = source.as_bytes();
                let before = expr
                    .op
                    .span
                    .start
                    .checked_sub(1)
                    .and_then(|index| bytes.get(index));
                let after = bytes.get(expr.op.span.end);
                if ![before, after]
                    .into_iter()
                    .all(|byte| byte.is_some_and(u8::is_ascii_whitespace))
                {
                    return Err(Issue::Invalid);
                }
            }
            let left = evaluate(&expr.left, source, font, viewport)?;
            let right = evaluate(&expr.right, source, font, viewport)?;
            match (&expr.op.kind, left, right) {
                (Op::Plus, left, right) => left.add(right)?,
                (Op::Minus, left, right) => left.add(right.scale(-1.0))?,
                (Op::Multiply, Value::Number(factor), value)
                | (Op::Multiply, value, Value::Number(factor)) => value.scale(factor),
                (Op::Division, value, Value::Number(divisor)) if divisor != 0.0 => {
                    value.scale(1.0 / divisor)
                }
                (Op::Division, _, Value::Number(_)) => return Err(Issue::Unsupported),
                _ => return Err(Issue::Invalid),
            }
        }
        _ => return Err(Issue::Unsupported),
    };
    let bounded = |value: f32, limit| value.is_finite() && value.abs() <= limit;
    let fits = match value {
        Value::Number(value) => bounded(value, MAX_RESULT),
        Value::Length { points, percent } => {
            points.is_none_or(|value| bounded(value, MAX_RESULT))
                && percent.is_none_or(|value| bounded(value, 10_000.0))
        }
    };
    if !fits {
        return Err(Issue::Limit);
    }
    Ok(value)
}

pub(super) fn valid(source: &str, percentages: bool) -> bool {
    match read(source, 12.0, [100.0; 2]) {
        Ok(Value::Number(_)) | Err(Issue::Invalid) => false,
        Ok(Value::Length {
            percent: Some(_), ..
        }) => percentages,
        _ => true,
    }
}

pub(super) fn resolve(
    source: &str,
    font: f32,
    viewport: [f32; 2],
    signed: bool,
) -> Result<Resolved, EngineError> {
    let (value, unit) = match read(source, font, viewport) {
        Ok(Value::Length {
            points: Some(points),
            percent: Some(percent),
        }) => {
            return Ok(Resolved::Mixed(MixedLength {
                points,
                percent,
                signed,
            }));
        }
        Ok(Value::Length {
            points: Some(value),
            percent: None,
        }) => (value, "pt"),
        Ok(Value::Length {
            points: None,
            percent: Some(value),
        }) => (value, "%"),
        Err(Issue::Limit) => return Err(limited("EPUB CSS calc exceeds its expression budget")),
        _ => return Err(unsupported("unsupported EPUB CSS calc expression")),
    };
    let value = if signed { value } else { value.max(0.0) };
    Ok(Resolved::Simple(format!("{value}{unit}")))
}
