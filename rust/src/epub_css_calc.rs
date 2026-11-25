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

#[path = "epub_css_trig.rs"]
mod trigonometry;
use trigonometry::{Input, Trig};
#[path = "epub_css_exp.rs"]
mod exponential;
use exponential::{Interval, Operation};

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
    Reciprocal(Box<Expression>, Interval),
    AffineRatio {
        numerator: [f32; 2],
        denominator: [f32; 2],
        bounds: Interval,
    },
    Step(Step, Box<Expression>, Box<Expression>),
    Trig(Trig, Box<Expression>),
    Atan2(Box<Expression>, Box<Expression>),
    Exponential(Operation, Box<Expression>, Box<Expression>, Interval),
    Hypot(Vec<Expression>),
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
            Self::Reciprocal(value, _) => value.at(basis).recip(),
            Self::AffineRatio {
                numerator,
                denominator,
                ..
            } => {
                // CSS percentage sizing has a nonnegative containing extent.
                // Evaluate coefficients in f64 so a tiny nonzero basis can
                // still cancel between proportional numerator and denominator.
                let extent = f64::from(basis.max(0.0)) / 100.0;
                let linear =
                    |&[points, percent]: &[f32; 2]| f64::from(points) + f64::from(percent) * extent;
                (linear(numerator) / linear(denominator)) as f32
            }
            Self::Step(step, value, interval) => step.evaluate(value.at(basis), interval.at(basis)),
            Self::Trig(operation, value) => operation.evaluate(value.at(basis)),
            Self::Atan2(y, x) => trigonometry::atan2(y.at(basis), x.at(basis)),
            Self::Exponential(operation, value, parameter, _) => {
                operation.evaluate(value.at(basis), parameter.at(basis))
            }
            Self::Hypot(values) => exponential::hypot(values.iter().map(|value| value.at(basis))),
        }
    }

    fn scalar_bounds(&self) -> Option<Interval> {
        Some(match self {
            Self::Linear(value, None) => {
                Interval::point(value.filter(|value| !value.is_nan()).unwrap_or_default())
            }
            Self::Linear(_, Some(_)) => return None,
            Self::Sum(one, other) => {
                let one = one.scalar_bounds()?;
                let other = other.scalar_bounds()?;
                Interval {
                    low: one.low + other.low,
                    high: one.high + other.high,
                }
            }
            Self::Scale(value, factor) => value.scalar_bounds()?.product(Interval::point(*factor)),
            Self::Compare(operation, values) => {
                let mut values = values.iter();
                let mut result = values.next()?.scalar_bounds()?;
                for value in values {
                    let value = value.scalar_bounds()?;
                    result.low = operation.choose(result.low, value.low);
                    result.high = operation.choose(result.high, value.high);
                }
                result
            }
            Self::Abs(value) => value.scalar_bounds()?.absolute(),
            Self::Sign(_) => Interval::signed(1.0),
            Self::Product(one, other) => one.scalar_bounds()?.product(other.scalar_bounds()?),
            Self::Reciprocal(_, bounds) | Self::AffineRatio { bounds, .. } => *bounds,
            Self::Step(operation, value, interval) => {
                let bound = interval.scalar_bounds()?.absolute().high;
                match operation {
                    Step::Round(_) => {
                        Interval::signed(bound + value.scalar_bounds()?.absolute().high)
                    }
                    Step::Mod | Step::Rem => Interval::signed(bound),
                }
            }
            Self::Trig(operation, value) => {
                Interval::signed(operation.bound(value.scalar_bounds()?.absolute().high)?)
            }
            Self::Atan2(_, _) => Interval::signed(180.0),
            Self::Exponential(_, _, _, bounds) => *bounds,
            Self::Hypot(values) => {
                let values = values
                    .iter()
                    .map(|value| value.scalar_bounds().map(Interval::absolute))
                    .collect::<Option<Vec<_>>>()?;
                Interval {
                    low: exponential::hypot(values.iter().map(|value| value.low)),
                    high: exponential::hypot(values.iter().map(|value| value.high)),
                }
            }
        })
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

    fn reciprocal(self) -> Result<Self, Issue> {
        if let Some(value) = self.fixed() {
            return if value == 0.0 {
                Err(Issue::Unsupported)
            } else {
                Ok(Self::constant(value.recip()))
            };
        }
        let bounds = self.expression.scalar_bounds().ok_or(Issue::Unsupported)?;
        if !bounds.low.is_finite()
            || !bounds.high.is_finite()
            || bounds.low <= 0.0 && bounds.high >= 0.0
        {
            return Err(Issue::Unsupported);
        }
        let bounds = Interval {
            low: bounds.high.recip(),
            high: bounds.low.recip(),
        };
        Ok(Self {
            expression: Expression::Reciprocal(Box::new(self.expression), bounds),
            point_bound: Some(bounds.absolute().high),
            percent_bound: None,
            relative_scalar: true,
            nodes: self.nodes + 1,
        })
    }

    fn affine_ratio(self, denominator: Self) -> Result<Self, Issue> {
        let coefficients = |expression: Expression| match expression {
            Expression::Linear(points, percent) => {
                let coefficients = [points.unwrap_or_default(), percent.unwrap_or_default()];
                coefficients
                    .into_iter()
                    .all(f32::is_finite)
                    .then_some(coefficients)
            }
            _ => None,
        };
        let numerator = coefficients(self.expression).ok_or(Issue::Unsupported)?;
        let bottom = coefficients(denominator.expression).ok_or(Issue::Unsupported)?;
        let [fixed, relative] = bottom;
        if fixed != 0.0 && relative != 0.0 && fixed.signum() != relative.signum()
            || (fixed == 0.0 && numerator[0] != 0.0)
            || (relative == 0.0 && numerator[1] != 0.0)
        {
            return Err(Issue::Unsupported);
        }
        // When the denominator's terms share one sign, each quotient at a
        // positive basis lies between the two coefficient ratios. A zero
        // coefficient shared by numerator and denominator adds no endpoint.
        let ratio = |top: f32, bottom: f32| {
            (bottom != 0.0).then(|| (f64::from(top) / f64::from(bottom)) as f32)
        };
        let origin = ratio(numerator[0], fixed);
        let infinity = ratio(numerator[1], relative);
        let lo = origin.or(infinity).ok_or(Issue::Unsupported)?;
        let hi = infinity.unwrap_or(lo);
        let bounds = Interval {
            low: lo.min(hi),
            high: lo.max(hi),
        };
        Ok(Self {
            expression: Expression::AffineRatio {
                numerator,
                denominator: bottom,
                bounds,
            },
            point_bound: Some(bounds.absolute().high),
            percent_bound: None,
            relative_scalar: true,
            nodes: self.nodes + denominator.nodes + 1,
        })
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
    Angle(Calculation),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Measure {
    Number,
    Length,
    Angle,
}

impl Measure {
    fn supply(self, value: Calculation) -> Value {
        match self {
            Self::Number => Value::Number(value),
            Self::Length => Value::Length(value),
            Self::Angle => Value::Angle(value),
        }
    }
}

impl Value {
    fn number(value: f32) -> Self {
        Self::Number(Calculation::constant(value))
    }

    fn split(self) -> (Measure, Calculation) {
        match self {
            Self::Number(value) => (Measure::Number, value),
            Self::Length(value) => (Measure::Length, value),
            Self::Angle(value) => (Measure::Angle, value),
        }
    }

    fn map(self, map: impl FnOnce(Calculation) -> Calculation) -> Self {
        let (kind, value) = self.split();
        kind.supply(map(value))
    }

    fn combine(
        self,
        other: Self,
        combine: impl FnOnce(Calculation, Calculation) -> Calculation,
    ) -> Result<Self, Issue> {
        let (kind, one) = self.split();
        let (other_kind, other) = other.split();
        if kind != other_kind {
            return Err(Issue::Invalid);
        }
        Ok(kind.supply(combine(one, other)))
    }

    fn scale(self, factor: f32) -> Self {
        self.map(|value| value.scale(factor))
    }

    fn add(self, other: Self) -> Result<Self, Issue> {
        self.combine(other, Calculation::add)
    }

    fn divide(self, other: Self) -> Result<Self, Issue> {
        let (kind, value) = self.split();
        let (denominator_kind, denominator) = other.split();
        if denominator_kind == Measure::Number {
            return Ok(kind.supply(value.multiply(denominator.reciprocal()?)));
        }
        if kind != denominator_kind {
            return Err(Issue::Invalid);
        }
        let ratio = if value.percent_bound.is_some() || denominator.percent_bound.is_some() {
            value.affine_ratio(denominator)?
        } else {
            value.multiply(denominator.reciprocal()?)
        };
        Ok(Self::Number(ratio))
    }

    fn compare(comparison: Comparison, mut values: Vec<Self>) -> Result<Self, Issue> {
        if values.len() == 1 {
            return values.pop().ok_or(Issue::Invalid);
        }
        let kind = match values.first() {
            Some(Self::Number(_)) => Measure::Number,
            Some(Self::Length(_)) => Measure::Length,
            Some(Self::Angle(_)) => Measure::Angle,
            None => return Err(Issue::Invalid),
        };
        let mut parts = Vec::with_capacity(values.len());
        let mut nodes = 1;
        let mut point_bound = None::<f32>;
        let mut percent_bound = None::<f32>;
        let mut relative_scalar = false;
        for value in values {
            let (current_kind, value) = value.split();
            if current_kind != kind {
                return Err(Issue::Invalid);
            }
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
        Ok(kind.supply(Calculation {
            expression: Expression::Compare(comparison, parts),
            point_bound,
            percent_bound,
            relative_scalar,
            nodes,
        }))
    }

    fn sign(self) -> Self {
        let (_, value) = self.split();
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
        self.combine(interval, |value, interval| {
            value.stepped(operation, interval)
        })
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
            "calc", "min", "max", "clamp", "round", "mod", "rem", "abs", "sign", "sin", "cos",
            "tan", "asin", "acos", "atan", "atan2", "pow", "sqrt", "exp", "log", "hypot",
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
        ComponentValue::Dimension(dimension) if dimension.kind == DimensionKind::Angle => {
            let degrees = trigonometry::degrees(dimension.value.value, &dimension.unit.name)
                .ok_or(Issue::Invalid)?;
            Value::Angle(Calculation::constant(degrees))
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
                (Op::Division, value, divisor) => value.divide(divisor)?,
                _ => return Err(Issue::Invalid),
            }
        }
        _ => return Err(Issue::Unsupported),
    };
    let (kind, calculation) = value.split();
    let value = if kind != Measure::Length {
        calculation
            .fixed()
            .map_or(calculation, Calculation::constant)
    } else {
        calculation
    };
    let bounded = |value: f32, limit| value.is_finite() && value.abs() <= limit;
    let fits = value
        .point_bound
        .is_none_or(|value| bounded(value, MAX_RESULT))
        && value
            .percent_bound
            .is_none_or(|value| bounded(value, MAX_PERCENT));
    if !fits {
        return Err(Issue::Limit);
    }
    Ok(kind.supply(value))
}

fn comma(arg: &ComponentValue<'_>) -> bool {
    matches!(arg, ComponentValue::Delimiter(delimiter) if delimiter.kind == DelimiterKind::Comma)
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
    if ["sin", "cos", "tan", "asin", "acos", "atan", "atan2"]
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
    {
        return evaluate_trig(&name.to_ascii_lowercase(), args, parse);
    }
    if ["pow", "sqrt", "exp", "log", "hypot"]
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
    {
        return evaluate_exponential(&name.to_ascii_lowercase(), args, parse);
    }
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
    Value::compare(comparison, comma_arguments(args, parse)?)
}

fn comma_arguments(
    args: &[ComponentValue<'_>],
    parse: impl Fn(&ComponentValue<'_>) -> Result<Value, Issue>,
) -> Result<Vec<Value>, Issue> {
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
    Ok(values)
}

fn stepped_function(
    name: &str,
    args: &[ComponentValue<'_>],
    parse: impl Fn(&ComponentValue<'_>) -> Result<Value, Issue>,
) -> Result<Value, Issue> {
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

fn evaluate_trig(
    name: &str,
    args: &[ComponentValue<'_>],
    parse: impl Fn(&ComponentValue<'_>) -> Result<Value, Issue>,
) -> Result<Value, Issue> {
    if name == "atan2" {
        let [y, separator, x] = args else {
            return Err(Issue::Invalid);
        };
        if !comma(separator) {
            return Err(Issue::Invalid);
        }
        let (y_kind, y) = parse(y)?.split();
        let (x_kind, x) = parse(x)?.split();
        if y_kind != x_kind {
            return Err(Issue::Invalid);
        }
        let relative = y.uses_basis() || x.uses_basis();
        return Ok(Value::Angle(Calculation {
            expression: Expression::Atan2(Box::new(y.expression), Box::new(x.expression)),
            point_bound: Some(180.0),
            percent_bound: None,
            relative_scalar: relative,
            nodes: y.nodes + x.nodes + 1,
        }));
    }
    let [argument] = args else {
        return Err(Issue::Invalid);
    };
    let (kind, value) = parse(argument)?.split();
    let input = match kind {
        Measure::Number => Input::Radians,
        Measure::Angle => Input::Degrees,
        Measure::Length => return Err(Issue::Invalid),
    };
    let (operation, output) = match name {
        "sin" => (Trig::Sin(input), Measure::Number),
        "cos" => (Trig::Cos(input), Measure::Number),
        "tan" => (Trig::Tan(input), Measure::Number),
        "asin" if kind == Measure::Number => (Trig::Asin, Measure::Angle),
        "acos" if kind == Measure::Number => (Trig::Acos, Measure::Angle),
        "atan" if kind == Measure::Number => (Trig::Atan, Measure::Angle),
        _ => return Err(Issue::Invalid),
    };
    if let Some(fixed) = value.fixed() {
        let result = operation.evaluate(fixed);
        if result.is_infinite() {
            return Err(Issue::Unsupported);
        }
        return Ok(output.supply(Calculation::constant(result)));
    }
    let bound = operation
        .bound(value.point_bound.unwrap_or_default())
        .ok_or(Issue::Unsupported)?;
    Ok(output.supply(Calculation {
        expression: Expression::Trig(operation, Box::new(value.expression)),
        point_bound: Some(bound),
        percent_bound: None,
        relative_scalar: true,
        nodes: value.nodes + 1,
    }))
}

fn evaluate_exponential(
    name: &str,
    args: &[ComponentValue<'_>],
    parse: impl Fn(&ComponentValue<'_>) -> Result<Value, Issue>,
) -> Result<Value, Issue> {
    let arguments = comma_arguments(args, parse)?;
    if name == "hypot" {
        return evaluate_hypot(arguments);
    }
    let operation = match (name, arguments.len()) {
        ("pow", 2) => Operation::Power,
        ("sqrt", 1) => Operation::Root,
        ("exp", 1) => Operation::Exp,
        ("log", 1) => Operation::Ln,
        ("log", 2) => Operation::Log,
        _ => return Err(Issue::Invalid),
    };
    let mut arguments = arguments.into_iter();
    let Some(Value::Number(value)) = arguments.next() else {
        return Err(Issue::Invalid);
    };
    let parameter = match arguments.next() {
        None => Calculation::constant(0.0),
        Some(Value::Number(parameter)) => parameter,
        _ => return Err(Issue::Invalid),
    };
    if let (Some(value), Some(parameter)) = (value.fixed(), parameter.fixed()) {
        let result = operation.evaluate(value, parameter);
        if result.is_infinite() {
            return Err(Issue::Unsupported);
        }
        return Ok(Value::number(result));
    }
    let bounds = |value: &Calculation| {
        value
            .expression
            .scalar_bounds()
            .unwrap_or_else(|| Interval::signed(value.point_bound.unwrap_or_default()))
    };
    let range = operation
        .bounds(bounds(&value), bounds(&parameter))
        .ok_or(Issue::Unsupported)?;
    Ok(Value::Number(Calculation {
        expression: Expression::Exponential(
            operation,
            Box::new(value.expression),
            Box::new(parameter.expression),
            range,
        ),
        point_bound: Some(range.absolute().high),
        percent_bound: None,
        relative_scalar: true,
        nodes: value.nodes + parameter.nodes + 1,
    }))
}

fn evaluate_hypot(arguments: Vec<Value>) -> Result<Value, Issue> {
    let mut arguments = arguments.into_iter().map(Value::split);
    let (kind, first) = arguments.next().ok_or(Issue::Invalid)?;
    let mut points = Vec::new();
    let mut percentages = Vec::new();
    let mut expressions = Vec::new();
    let mut relative_scalar = false;
    let mut nodes = 1;
    for (other, value) in std::iter::once((kind, first)).chain(arguments) {
        if other != kind {
            return Err(Issue::Invalid);
        }
        points.extend(value.point_bound);
        percentages.extend(value.percent_bound);
        relative_scalar |= value.relative_scalar;
        nodes += value.nodes;
        expressions.push(value.expression);
    }
    let norm = |values: Vec<f32>| (!values.is_empty()).then(|| exponential::hypot(values));
    let calculation = Calculation {
        expression: Expression::Hypot(expressions),
        point_bound: norm(points),
        percent_bound: norm(percentages),
        relative_scalar,
        nodes,
    };
    Ok(kind.supply(
        calculation
            .fixed()
            .map_or(calculation, Calculation::constant),
    ))
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
        Ok(Value::Number(_) | Value::Angle(_)) | Err(Issue::Invalid) => false,
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
