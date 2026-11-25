//! Resolve bounded CSS length arithmetic and comparisons for native EPUB layout.
use raffia::{
    ast::{
        CalcOperatorKind as Op, ComponentValue, Declaration, DelimiterKind, DimensionKind,
        FunctionName,
    },
    Parser, Syntax,
};
use taffy::CalcHandle;

use super::{ident, points};
use crate::{
    epub::{charge, limited, unsupported},
    EngineError,
};

const MAX_EXPRESSION_BYTES: usize = 512;
const MAX_RESULT: f32 = 25_000.0;
const MAX_PERCENT: f32 = 10_000.0;
const MAX_CHAPTER_NODES: usize = 262_144;

#[derive(Clone, Copy)]
enum Comparison {
    Min,
    Max,
}

impl Comparison {
    fn choose(self, one: f32, other: f32) -> f32 {
        match self {
            Self::Min => one.min(other),
            Self::Max => one.max(other),
        }
    }
}

enum Expression {
    Linear(Option<f32>, Option<f32>),
    Sum(Box<Expression>, Box<Expression>),
    Scale(Box<Expression>, f32),
    Compare(Comparison, Vec<Expression>),
}

impl Expression {
    fn at(&self, basis: f32) -> f32 {
        match self {
            Self::Linear(points, percent) => {
                points.unwrap_or_default() + basis * (percent.unwrap_or_default() / 100.0)
            }
            Self::Sum(left, right) => left.at(basis) + right.at(basis),
            Self::Scale(value, factor) => value.at(basis) * factor,
            Self::Compare(comparison, values) => values
                .iter()
                .map(|value| value.at(basis))
                .reduce(|one, other| comparison.choose(one, other))
                .unwrap_or_default(),
        }
    }
}

// An optional bound records which units were present even if their values cancel.
struct Length {
    expression: Expression,
    point_bound: Option<f32>,
    percent_bound: Option<f32>,
    nodes: usize,
}

impl Length {
    fn linear(points: Option<f32>, percent: Option<f32>) -> Self {
        Self {
            expression: Expression::Linear(points, percent),
            point_bound: points.map(f32::abs),
            percent_bound: percent.map(f32::abs),
            nodes: 1,
        }
    }

    fn scale(self, factor: f32) -> Self {
        if let Expression::Linear(points, percent) = self.expression {
            return Self::linear(points.map(|n| n * factor), percent.map(|n| n * factor));
        }
        Self {
            expression: Expression::Scale(Box::new(self.expression), factor),
            point_bound: self.point_bound.map(|n| n * factor.abs()),
            percent_bound: self.percent_bound.map(|n| n * factor.abs()),
            nodes: self.nodes + 1,
        }
    }

    fn add(self, other: Self) -> Self {
        let add = |a: Option<f32>, b: Option<f32>| {
            a.or(b)
                .map(|_| a.unwrap_or_default() + b.unwrap_or_default())
        };
        match (self.expression, other.expression) {
            (
                Expression::Linear(points, percent),
                Expression::Linear(other_points, other_percent),
            ) => Self::linear(add(points, other_points), add(percent, other_percent)),
            (left, right) => Self {
                expression: Expression::Sum(Box::new(left), Box::new(right)),
                point_bound: add(self.point_bound, other.point_bound),
                percent_bound: add(self.percent_bound, other.percent_bound),
                nodes: self.nodes + other.nodes + 1,
            },
        }
    }
}

enum Value {
    Number(f32),
    Length(Length),
}

impl Value {
    fn scale(self, factor: f32) -> Self {
        match self {
            Self::Number(value) => Self::Number(value * factor),
            Self::Length(value) => Self::Length(value.scale(factor)),
        }
    }

    fn add(self, other: Self) -> Result<Self, Issue> {
        match (self, other) {
            (Self::Number(one), Self::Number(other)) => Ok(Self::Number(one + other)),
            (Self::Length(one), Self::Length(other)) => Ok(Self::Length(one.add(other))),
            _ => Err(Issue::Invalid),
        }
    }

    fn compare(comparison: Comparison, mut values: Vec<Self>) -> Result<Self, Issue> {
        if values.len() == 1 {
            return values.pop().ok_or(Issue::Invalid);
        }
        match values.first() {
            Some(Self::Number(_)) => {
                let mut numbers = values.into_iter().map(|value| match value {
                    Self::Number(n) => Ok(n),
                    Self::Length(_) => Err(Issue::Invalid),
                });
                let first = numbers.next().ok_or(Issue::Invalid)??;
                numbers
                    .try_fold(first, |one, other| Ok(comparison.choose(one, other?)))
                    .map(Self::Number)
            }
            Some(Self::Length(_)) => {
                let mut parts = Vec::with_capacity(values.len());
                let mut nodes = 1;
                let mut point_bound = None::<f32>;
                let mut percent_bound = None::<f32>;
                for value in values {
                    let Self::Length(length) = value else {
                        return Err(Issue::Invalid);
                    };
                    point_bound = point_bound
                        .into_iter()
                        .chain(length.point_bound)
                        .reduce(f32::max);
                    percent_bound = percent_bound
                        .into_iter()
                        .chain(length.percent_bound)
                        .reduce(f32::max);
                    nodes += length.nodes;
                    parts.push(length.expression);
                }
                Ok(Self::Length(Length {
                    expression: Expression::Compare(comparison, parts),
                    point_bound,
                    percent_bound,
                    nodes,
                }))
            }
            None => Err(Issue::Invalid),
        }
    }
}

pub(super) struct CalculatedLength {
    value: Length,
    signed: bool,
}

#[derive(Default)]
pub(in crate::epub) struct CalculationTable {
    values: Vec<CalculatedLength>,
    nodes: usize,
}

impl CalculationTable {
    pub(super) fn register(&mut self, length: CalculatedLength) -> Result<CalcHandle, EngineError> {
        charge(
            &mut self.nodes,
            length.value.nodes,
            MAX_CHAPTER_NODES,
            "EPUB CSS chapter calculations exceed their node budget",
        )?;
        self.values.push(length);
        // CompactLength reserves the zero sentinel and three low bits.
        Ok(CalcHandle::from_bits((self.values.len() as u64) << 3))
    }

    fn get(&self, handle: CalcHandle) -> &CalculatedLength {
        &self.values[((handle.to_bits() >> 3) - 1) as usize]
    }

    pub(in crate::epub) fn at(&self, handle: CalcHandle, basis: f32) -> f32 {
        let length = self.get(handle);
        let value = length.value.expression.at(basis);
        if length.signed {
            value
        } else {
            value.max(0.0)
        }
    }

    pub(in crate::epub) fn has_fixed_part(&self, handle: CalcHandle) -> bool {
        self.get(handle).value.point_bound.unwrap_or_default() > 0.0
    }
}

pub(super) enum Resolved {
    Points(f32),
    Percent(f32),
    Deferred(CalculatedLength),
}

enum Issue {
    Invalid,
    Unsupported,
    Limit,
}

pub(super) fn is_math(value: &str) -> bool {
    value.split_once('(').is_some_and(|(name, _)| {
        ["calc", "min", "max", "clamp"]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword))
    })
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
        ComponentValue::Percentage(percent) => {
            Value::Length(Length::linear(None, Some(percent.value.value)))
        }
        ComponentValue::Dimension(dimension) if dimension.kind == DimensionKind::Length => {
            let unit = dimension.unit.name.to_ascii_lowercase();
            let raw = format!("{}{unit}", dimension.value.raw);
            let resolved = points(&raw, font, viewport).ok_or(Issue::Unsupported)?;
            Value::Length(Length::linear(Some(resolved), None))
        }
        ComponentValue::Dimension(_) => return Err(Issue::Invalid),
        ComponentValue::Function(function) => {
            let FunctionName::Ident(name) = &function.name else {
                return Err(Issue::Unsupported);
            };
            let name = ident(name).ok_or(Issue::Unsupported)?;
            evaluate_function(name, &function.args, |arg| {
                evaluate(arg, source, font, viewport)
            })?
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
    let fits = match &value {
        Value::Number(value) => bounded(*value, MAX_RESULT),
        Value::Length(length) => {
            length
                .point_bound
                .is_none_or(|value| bounded(value, MAX_RESULT))
                && length
                    .percent_bound
                    .is_none_or(|value| bounded(value, MAX_PERCENT))
        }
    };
    if !fits {
        return Err(Issue::Limit);
    }
    Ok(value)
}

fn evaluate_function(
    name: &str,
    args: &[ComponentValue<'_>],
    parse: impl Fn(&ComponentValue<'_>) -> Result<Value, Issue>,
) -> Result<Value, Issue> {
    if name.eq_ignore_ascii_case("calc") {
        let [arg] = args else {
            return Err(Issue::Invalid);
        };
        return parse(arg);
    }
    let comma = |arg: &ComponentValue<'_>| matches!(arg, ComponentValue::Delimiter(delimiter) if delimiter.kind == DelimiterKind::Comma);
    if name.eq_ignore_ascii_case("clamp") {
        let [lower, first, center, second, upper] = args else {
            return Err(Issue::Invalid);
        };
        if !comma(first) || !comma(second) {
            return Err(Issue::Invalid);
        }
        let none = |arg: &ComponentValue<'_>| {
            matches!(arg, ComponentValue::InterpolableIdent(name)
            if ident(name).is_some_and(|name| name.eq_ignore_ascii_case("none")))
        };
        let mut center = parse(center)?;
        if !none(upper) {
            center = Value::compare(Comparison::Min, vec![center, parse(upper)?])?;
        }
        if !none(lower) {
            center = Value::compare(Comparison::Max, vec![parse(lower)?, center])?;
        }
        return Ok(center);
    }
    let comparison = if name.eq_ignore_ascii_case("min") {
        Comparison::Min
    } else if name.eq_ignore_ascii_case("max") {
        Comparison::Max
    } else {
        return Err(Issue::Unsupported);
    };
    let [first, tail @ ..] = args else {
        return Err(Issue::Invalid);
    };
    let (pairs, remainder) = tail.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(Issue::Invalid);
    }
    let mut values = vec![parse(first)?];
    for [separator, argument] in pairs {
        if !comma(separator) {
            return Err(Issue::Invalid);
        }
        values.push(parse(argument)?);
    }
    Value::compare(comparison, values)
}

pub(super) fn valid(source: &str, percentages: bool) -> bool {
    match read(source, 12.0, [100.0; 2]) {
        Ok(Value::Number(_)) | Err(Issue::Invalid) => false,
        Ok(Value::Length(length)) => length.percent_bound.is_none() || percentages,
        _ => true,
    }
}

pub(super) fn resolve(
    source: &str,
    font: f32,
    viewport: [f32; 2],
    signed: bool,
) -> Result<Resolved, EngineError> {
    let clamp = |value: f32| if signed { value } else { value.max(0.0) };
    match read(source, font, viewport) {
        Ok(Value::Length(length)) if length.percent_bound.is_none() => {
            Ok(Resolved::Points(clamp(length.expression.at(0.0))))
        }
        Ok(Value::Length(length)) if length.point_bound.is_none() => {
            Ok(Resolved::Percent(clamp(length.expression.at(100.0))))
        }
        Ok(Value::Length(value)) => Ok(Resolved::Deferred(CalculatedLength { value, signed })),
        Err(Issue::Limit) => Err(limited("EPUB CSS calc exceeds its expression budget")),
        _ => Err(unsupported("unsupported EPUB CSS calc expression")),
    }
}
