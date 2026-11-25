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

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Number,
    Length,
    Percent,
}

#[derive(Clone, Copy)]
struct Value(Kind, f32);

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
    use Kind::*;
    let value = match expr {
        ComponentValue::Number(number) => Value(Number, number.value),
        ComponentValue::Percentage(percent) => Value(Percent, percent.value.value),
        ComponentValue::Dimension(dimension) if dimension.kind == DimensionKind::Length => {
            let unit = dimension.unit.name.to_ascii_lowercase();
            let raw = format!("{}{unit}", dimension.value.raw);
            let resolved = points(&raw, font, viewport).ok_or(Issue::Unsupported)?;
            Value(Length, resolved)
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
            let Value(left_kind, left) = evaluate(&expr.left, source, font, viewport)?;
            let Value(right_kind, right) = evaluate(&expr.right, source, font, viewport)?;
            match (&expr.op.kind, left_kind, right_kind) {
                (Op::Plus | Op::Minus, a, b) if a == b => {
                    let sign = if expr.op.kind == Op::Plus { 1.0 } else { -1.0 };
                    Value(a, left + sign * right)
                }
                (Op::Plus | Op::Minus, Length, Percent)
                | (Op::Plus | Op::Minus, Percent, Length) => return Err(Issue::Unsupported),
                (Op::Multiply, Number, kind) | (Op::Multiply, kind, Number) => {
                    Value(kind, left * right)
                }
                (Op::Division, kind, Number) if right != 0.0 => Value(kind, left / right),
                (Op::Division, _, Number) => return Err(Issue::Unsupported),
                _ => return Err(Issue::Invalid),
            }
        }
        _ => return Err(Issue::Unsupported),
    };
    let limit = if value.0 == Percent {
        10_000.0
    } else {
        MAX_RESULT
    };
    if !value.1.is_finite() || value.1.abs() > limit {
        return Err(Issue::Limit);
    }
    Ok(value)
}

pub(super) fn valid(source: &str, percentages: bool) -> bool {
    match read(source, 12.0, [100.0; 2]) {
        Ok(Value(Kind::Number, _)) | Err(Issue::Invalid) => false,
        Ok(Value(Kind::Percent, _)) => percentages,
        _ => true,
    }
}

pub(super) fn resolve(
    source: &str,
    font: f32,
    viewport: [f32; 2],
    signed: bool,
) -> Result<String, EngineError> {
    let (value, unit) = match read(source, font, viewport) {
        Ok(Value(Kind::Length, value)) => (value, "pt"),
        Ok(Value(Kind::Percent, value)) => (value, "%"),
        Err(Issue::Limit) => return Err(limited("EPUB CSS calc exceeds its expression budget")),
        _ => return Err(unsupported("unsupported EPUB CSS calc expression")),
    };
    let value = if signed { value } else { value.max(0.0) };
    Ok(format!("{value}{unit}"))
}
