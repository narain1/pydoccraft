//! Resolve bounded CSS math expressions for native EPUB layout.
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
        if one.is_nan() || other.is_nan() {
            return f32::NAN;
        }
        match self {
            Self::Min => one.min(other),
            Self::Max => one.max(other),
        }
    }
}

#[derive(Clone, Copy)]
enum Rounding {
    Nearest,
    Up,
    Down,
    ToZero,
}

#[derive(Clone, Copy)]
enum Step {
    Round(Rounding),
    Mod,
    Rem,
}

impl Step {
    fn evaluate(self, value: f32, step: f32) -> f32 {
        if !value.is_finite() || !step.is_finite() || step == 0.0 {
            return f32::NAN;
        }
        let remainder = value % step;
        match self {
            Self::Round(strategy) => {
                if remainder == 0.0 {
                    return value;
                }
                let step = f64::from(step).abs();
                let quotient = f64::from(value) / step;
                let multiple = match strategy {
                    Rounding::Nearest => (quotient + 0.5).floor(),
                    Rounding::Up => quotient.ceil(),
                    Rounding::Down => quotient.floor(),
                    Rounding::ToZero => quotient.trunc(),
                };
                let result = (multiple * step) as f32;
                if result == 0.0 {
                    0.0f32.copysign(value)
                } else {
                    result
                }
            }
            Self::Mod if remainder == 0.0 => 0.0f32.copysign(step),
            Self::Mod if remainder.is_sign_negative() != step.is_sign_negative() => {
                remainder + step
            }
            Self::Mod | Self::Rem => remainder,
        }
    }
}

enum Expression {
    Linear(Option<f32>, Option<f32>),
    Sum(Box<Expression>, Box<Expression>),
    Scale(Box<Expression>, f32),
    Compare(Comparison, Vec<Expression>),
    Abs(Box<Expression>),
    Sign(Box<Expression>),
    Product(Box<Expression>, Box<Expression>),
    Step(Step, Box<Expression>, Box<Expression>),
}

impl Expression {
    fn at(&self, basis: f32) -> f32 {
        match self {
            Self::Linear(points, None) => points.unwrap_or_default(),
            Self::Linear(None, Some(percent)) => basis * (percent / 100.0),
            Self::Linear(Some(points), Some(percent)) => points + basis * (percent / 100.0),
            Self::Sum(left, right) => left.at(basis) + right.at(basis),
            Self::Scale(value, factor) => value.at(basis) * factor,
            Self::Compare(comparison, values) => values
                .iter()
                .map(|value| value.at(basis))
                .reduce(|one, other| comparison.choose(one, other))
                .unwrap_or_default(),
            Self::Abs(value) => value.at(basis).abs(),
            Self::Sign(value) => sign(value.at(basis)),
            Self::Product(left, right) => left.at(basis) * right.at(basis),
            Self::Step(step, value, interval) => step.evaluate(value.at(basis), interval.at(basis)),
        }
    }
}

// An optional bound records which units were present even if their values cancel.
struct Calculation {
    expression: Expression,
    point_bound: Option<f32>,
    percent_bound: Option<f32>,
    relative_scalar: bool,
    nodes: usize,
}

fn sum_bounds(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    a.or(b)
        .map(|_| a.unwrap_or_default() + b.unwrap_or_default())
}

fn sign(value: f32) -> f32 {
    if value == 0.0 {
        value
    } else {
        value.signum()
    }
}

impl Calculation {
    fn constant(value: f32) -> Self {
        Self::linear(Some(value), None)
    }

    fn linear(points: Option<f32>, percent: Option<f32>) -> Self {
        // NaN from a zero step remains in the expression. Its eventual
        // top-level used value is zero, so it contributes no magnitude.
        let bound = |value: f32| if value.is_nan() { 0.0 } else { value.abs() };
        Self {
            expression: Expression::Linear(points, percent),
            point_bound: points.map(bound),
            percent_bound: percent.map(bound),
            relative_scalar: false,
            nodes: 1,
        }
    }

    fn uses_basis(&self) -> bool {
        self.percent_bound.is_some() || self.relative_scalar
    }

    fn fixed(&self) -> Option<f32> {
        (!self.uses_basis()).then(|| self.expression.at(0.0))
    }

    fn scale(self, factor: f32) -> Self {
        if let (Expression::Linear(points, percent), true) = (&self.expression, factor.is_finite())
        {
            return Self::linear(points.map(|n| n * factor), percent.map(|n| n * factor));
        }
        let bound = if factor.is_nan() { 0.0 } else { factor.abs() };
        Self {
            expression: Expression::Scale(Box::new(self.expression), factor),
            point_bound: self.point_bound.map(|n| n * bound),
            percent_bound: self.percent_bound.map(|n| n * bound),
            relative_scalar: self.relative_scalar,
            nodes: self.nodes + 1,
        }
    }

    fn add(self, other: Self) -> Self {
        match (self.expression, other.expression) {
            (
                Expression::Linear(points, percent),
                Expression::Linear(other_points, other_percent),
            ) => Self::linear(
                sum_bounds(points, other_points),
                sum_bounds(percent, other_percent),
            ),
            (left, right) => Self {
                expression: Expression::Sum(Box::new(left), Box::new(right)),
                point_bound: sum_bounds(self.point_bound, other.point_bound),
                percent_bound: sum_bounds(self.percent_bound, other.percent_bound),
                relative_scalar: self.relative_scalar || other.relative_scalar,
                nodes: self.nodes + other.nodes + 1,
            },
        }
    }

    fn multiply(self, factor: Self) -> Self {
        if let Some(factor) = factor.fixed() {
            return self.scale(factor);
        }
        let bound = factor.point_bound.unwrap_or_default();
        Self {
            expression: Expression::Product(Box::new(self.expression), Box::new(factor.expression)),
            point_bound: self.point_bound.map(|value| value * bound),
            percent_bound: self.percent_bound.map(|value| value * bound),
            relative_scalar: true,
            nodes: self.nodes + factor.nodes + 1,
        }
    }

    fn absolute(self) -> Self {
        Self {
            expression: Expression::Abs(Box::new(self.expression)),
            nodes: self.nodes + 1,
            ..self
        }
    }

    fn stepped(self, step: Step, interval: Self) -> Self {
        if let (Some(value), Some(interval)) = (self.fixed(), interval.fixed()) {
            return Self::constant(step.evaluate(value, interval));
        }
        let bound = |value, interval: Option<f32>| match step {
            Step::Round(_) => sum_bounds(value, interval),
            Step::Mod | Step::Rem => interval.or(value.map(|_| 0.0)),
        };
        Self {
            expression: Expression::Step(
                step,
                Box::new(self.expression),
                Box::new(interval.expression),
            ),
            point_bound: bound(self.point_bound, interval.point_bound),
            percent_bound: bound(self.percent_bound, interval.percent_bound),
            relative_scalar: self.relative_scalar || interval.relative_scalar,
            nodes: self.nodes + interval.nodes + 1,
        }
    }
}

enum Value {
    Number(Calculation),
    Length(Calculation),
}

impl Value {
    fn number(value: f32) -> Self {
        Self::Number(Calculation::constant(value))
    }

    fn map(self, map: impl FnOnce(Calculation) -> Calculation) -> Self {
        match self {
            Self::Number(value) => Self::Number(map(value)),
            Self::Length(value) => Self::Length(map(value)),
        }
    }

    fn scale(self, factor: f32) -> Self {
        self.map(|value| value.scale(factor))
    }

    fn add(self, other: Self) -> Result<Self, Issue> {
        match (self, other) {
            (Self::Number(one), Self::Number(other)) => Ok(Self::Number(one.add(other))),
            (Self::Length(one), Self::Length(other)) => Ok(Self::Length(one.add(other))),
            _ => Err(Issue::Invalid),
        }
    }

    fn compare(comparison: Comparison, mut values: Vec<Self>) -> Result<Self, Issue> {
        if values.len() == 1 {
            return values.pop().ok_or(Issue::Invalid);
        }
        let numeric = matches!(values.first(), Some(Self::Number(_)));
        let mut parts = Vec::with_capacity(values.len());
        let mut nodes = 1;
        let mut point_bound = None::<f32>;
        let mut percent_bound = None::<f32>;
        let mut relative_scalar = false;
        for value in values {
            let value = match value {
                Self::Number(value) if numeric => value,
                Self::Length(value) if !numeric => value,
                _ => return Err(Issue::Invalid),
            };
            point_bound = point_bound
                .into_iter()
                .chain(value.point_bound)
                .reduce(f32::max);
            percent_bound = percent_bound
                .into_iter()
                .chain(value.percent_bound)
                .reduce(f32::max);
            relative_scalar |= value.relative_scalar;
            nodes += value.nodes;
            parts.push(value.expression);
        }
        if parts.is_empty() {
            return Err(Issue::Invalid);
        }
        let value = Calculation {
            expression: Expression::Compare(comparison, parts),
            point_bound,
            percent_bound,
            relative_scalar,
            nodes,
        };
        Ok(if numeric {
            Self::Number(value)
        } else {
            Self::Length(value)
        })
    }

    fn sign(self) -> Self {
        let (Self::Number(value) | Self::Length(value)) = self;
        if let Some(fixed) = value.fixed() {
            if fixed.is_finite() {
                return Self::number(sign(fixed));
            }
        }
        Self::Number(Calculation {
            relative_scalar: value.uses_basis(),
            expression: Expression::Sign(Box::new(value.expression)),
            point_bound: Some(1.0),
            percent_bound: None,
            nodes: value.nodes + 1,
        })
    }

    fn step(self, operation: Step, interval: Self) -> Result<Self, Issue> {
        match (self, interval) {
            (Self::Number(value), Self::Number(interval)) => {
                Ok(Self::Number(value.stepped(operation, interval)))
            }
            (Self::Length(value), Self::Length(interval)) => {
                Ok(Self::Length(value.stepped(operation, interval)))
            }
            _ => Err(Issue::Invalid),
        }
    }
}

pub(super) struct CalculatedLength {
    value: Calculation,
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
        used_length(length.value.expression.at(basis), length.signed)
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
        [
            "calc", "min", "max", "clamp", "round", "mod", "rem", "abs", "sign",
        ]
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
        ComponentValue::Number(number) => Value::number(number.value),
        ComponentValue::Percentage(percent) => {
            Value::Length(Calculation::linear(None, Some(percent.value.value)))
        }
        ComponentValue::Dimension(dimension) if dimension.kind == DimensionKind::Length => {
            let unit = dimension.unit.name.to_ascii_lowercase();
            let raw = format!("{}{unit}", dimension.value.raw);
            let resolved = points(&raw, font, viewport).ok_or(Issue::Unsupported)?;
            Value::Length(Calculation::linear(Some(resolved), None))
        }
        ComponentValue::Dimension(_) => return Err(Issue::Invalid),
        ComponentValue::InterpolableIdent(name) => match ident(name).ok_or(Issue::Unsupported)? {
            name if name.eq_ignore_ascii_case("pi") => Value::number(std::f32::consts::PI),
            name if name.eq_ignore_ascii_case("e") => Value::number(std::f32::consts::E),
            _ => return Err(Issue::Unsupported),
        },
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
                | (Op::Multiply, value, Value::Number(factor)) => {
                    value.map(|value| value.multiply(factor))
                }
                (Op::Division, value, Value::Number(divisor)) => {
                    let divisor = divisor
                        .fixed()
                        .filter(|&value| value != 0.0)
                        .ok_or(Issue::Unsupported)?;
                    value.scale(1.0 / divisor)
                }
                _ => return Err(Issue::Invalid),
            }
        }
        _ => return Err(Issue::Unsupported),
    };
    let value = match value {
        Value::Number(scalar) => match scalar.fixed() {
            Some(number) => Value::number(number),
            None => Value::Number(scalar),
        },
        value => value,
    };
    let bounded = |value: f32, limit| value.is_finite() && value.abs() <= limit;
    let fits = match &value {
        Value::Number(length) | Value::Length(length) => {
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
    if name.eq_ignore_ascii_case("abs") || name.eq_ignore_ascii_case("sign") {
        let [arg] = args else {
            return Err(Issue::Invalid);
        };
        let value = parse(arg)?;
        return Ok(if name.eq_ignore_ascii_case("abs") {
            value.map(Calculation::absolute)
        } else {
            value.sign()
        });
    }
    if ["round", "mod", "rem"]
        .iter()
        .any(|key| name.eq_ignore_ascii_case(key))
    {
        return stepped_function(name, args, parse);
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

fn stepped_function(
    name: &str,
    args: &[ComponentValue<'_>],
    parse: impl Fn(&ComponentValue<'_>) -> Result<Value, Issue>,
) -> Result<Value, Issue> {
    let comma = |arg: &ComponentValue<'_>| matches!(arg, ComponentValue::Delimiter(delimiter) if delimiter.kind == DelimiterKind::Comma);
    let (step, args) = if name.eq_ignore_ascii_case("round") {
        let strategy = match args {
            [ComponentValue::InterpolableIdent(strategy), separator, rest @ ..]
                if comma(separator) =>
            {
                let strategy = ident(strategy).ok_or(Issue::Invalid)?;
                match strategy.to_ascii_lowercase().as_str() {
                    "nearest" => Some((Rounding::Nearest, rest)),
                    "up" => Some((Rounding::Up, rest)),
                    "down" => Some((Rounding::Down, rest)),
                    "to-zero" => Some((Rounding::ToZero, rest)),
                    _ => None,
                }
            }
            _ => None,
        };
        let (rounding, args) = strategy.unwrap_or((Rounding::Nearest, args));
        (Step::Round(rounding), args)
    } else if name.eq_ignore_ascii_case("mod") {
        (Step::Mod, args)
    } else {
        (Step::Rem, args)
    };
    match args {
        [value, separator, interval] if comma(separator) => {
            parse(value)?.step(step, parse(interval)?)
        }
        [value] if matches!(step, Step::Round(_)) => parse(value)?.step(step, Value::number(1.0)),
        _ => Err(Issue::Invalid),
    }
}

fn used_length(value: f32, signed: bool) -> f32 {
    // CSS censors NaN at the boundary of a top-level math expression.
    if value.is_nan() || !signed && value < 0.0 {
        0.0
    } else {
        value
    }
}

pub(super) fn valid(source: &str, percentages: bool) -> bool {
    match read(source, 12.0, [100.0; 2]) {
        Ok(Value::Number(_)) | Err(Issue::Invalid) => false,
        Ok(Value::Length(length)) => !length.uses_basis() || percentages,
        _ => true,
    }
}

pub(super) fn resolve(
    source: &str,
    font: f32,
    viewport: [f32; 2],
    signed: bool,
) -> Result<Resolved, EngineError> {
    match read(source, font, viewport) {
        Ok(Value::Length(length)) if !length.uses_basis() => Ok(Resolved::Points(used_length(
            length.expression.at(0.0),
            signed,
        ))),
        Ok(Value::Length(length)) if length.point_bound.is_none() && !length.relative_scalar => Ok(
            Resolved::Percent(used_length(length.expression.at(100.0), signed)),
        ),
        Ok(Value::Length(value)) => Ok(Resolved::Deferred(CalculatedLength { value, signed })),
        Err(Issue::Limit) => Err(limited("EPUB CSS calc exceeds its expression budget")),
        _ => Err(unsupported("unsupported EPUB CSS calc expression")),
    }
}
