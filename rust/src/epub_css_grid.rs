//! Parse the EPUB grid track profile and project it into the native layout tree.
use raffia::{
    ast::{ComponentValue, Declaration, Delimiter, DelimiterKind, DimensionKind, FunctionName},
    Parser, Spanned, Syntax,
};
use taffy::{
    style::{GridTemplateComponent, LengthPercentage, TrackSizingFunction},
    style_helpers::{auto, fit_content, fr, max_content, min_content},
};

use super::{calc, ident, points};
use crate::{
    epub::{invalid, limited, unsupported},
    EngineError,
};

const MAX_TRACKS: usize = 32;

#[derive(Clone, Copy)]
enum Length<'a> {
    Points(&'a str),
    Percent(f32),
    Calc(&'a str),
}

#[derive(Clone, Copy)]
enum Breadth<'a> {
    Fixed(Length<'a>),
    Auto,
    MinContent,
    MaxContent,
    Flex(f32),
}

#[derive(Clone, Copy)]
enum Track<'a> {
    Breadth(Breadth<'a>),
    MinMax(Breadth<'a>, Breadth<'a>),
    Fit(Length<'a>),
}

enum Issue {
    Invalid,
    Unsupported,
    Limit,
}

pub(super) fn valid(value: &str, components: &[ComponentValue<'_>]) -> bool {
    !matches!(parse(value, components), Err(Issue::Invalid))
}

fn parse<'a>(value: &'a str, components: &[ComponentValue<'_>]) -> Result<Vec<Track<'a>>, Issue> {
    let mut tracks = Vec::new();
    if value.eq_ignore_ascii_case("none") {
        return Ok(tracks);
    }
    let offset = components.first().ok_or(Issue::Invalid)?.span().start;
    let raw = |item: &ComponentValue<'_>| {
        let span = item.span();
        value.get(span.start.checked_sub(offset)?..span.end.checked_sub(offset)?)
    };
    for item in components {
        if let Some((_, args)) =
            function(item).filter(|(name, _)| name.eq_ignore_ascii_case("repeat"))
        {
            let [count, delimiter, list @ ..] = args else {
                return Err(Issue::Invalid);
            };
            if !comma(delimiter) || list.is_empty() {
                return Err(Issue::Invalid);
            }
            let count = match count {
                ComponentValue::Number(count) => {
                    let digits = count.raw.strip_prefix('+').unwrap_or(count.raw);
                    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                        return Err(Issue::Invalid);
                    }
                    digits.parse::<usize>().map_err(|_| Issue::Limit)?
                }
                ComponentValue::InterpolableIdent(name)
                    if ident(name).is_some_and(|name| {
                        matches!(name.to_ascii_lowercase().as_str(), "auto-fit" | "auto-fill")
                    }) =>
                {
                    return Err(Issue::Unsupported);
                }
                _ => return Err(Issue::Invalid),
            };
            if count == 0 {
                return Err(Issue::Invalid);
            }
            if count > (MAX_TRACKS - tracks.len()) / list.len() {
                return Err(Issue::Limit);
            }
            let repeated = list
                .iter()
                .map(|item| track(item, &raw))
                .collect::<Result<Vec<_>, _>>()?;
            for _ in 0..count {
                tracks.extend_from_slice(&repeated);
            }
        } else {
            if tracks.len() == MAX_TRACKS {
                return Err(Issue::Limit);
            }
            tracks.push(track(item, &raw)?);
        }
    }
    Ok(tracks)
}

fn function<'a, 's>(item: &'a ComponentValue<'s>) -> Option<(&'a str, &'a [ComponentValue<'s>])> {
    let ComponentValue::Function(node) = item else {
        return None;
    };
    let FunctionName::Ident(name) = &node.name else {
        return None;
    };
    Some((ident(name)?, &node.args))
}

fn comma(item: &ComponentValue<'_>) -> bool {
    matches!(
        item,
        ComponentValue::Delimiter(Delimiter {
            kind: DelimiterKind::Comma,
            ..
        })
    )
}

fn track<'a>(
    item: &ComponentValue<'_>,
    raw: &impl Fn(&ComponentValue<'_>) -> Option<&'a str>,
) -> Result<Track<'a>, Issue> {
    if let Some((name, args)) = function(item) {
        if name.eq_ignore_ascii_case("minmax") {
            let [minimum, separator, maximum] = args else {
                return Err(Issue::Invalid);
            };
            if !comma(separator) {
                return Err(Issue::Invalid);
            }
            let minimum = breadth(minimum, raw)?;
            if matches!(minimum, Breadth::Flex(_)) {
                return Err(Issue::Invalid);
            }
            return Ok(Track::MinMax(minimum, breadth(maximum, raw)?));
        }
        if name.eq_ignore_ascii_case("fit-content") {
            let [limit] = args else {
                return Err(Issue::Invalid);
            };
            let Breadth::Fixed(limit) = breadth(limit, raw)? else {
                return Err(Issue::Invalid);
            };
            return Ok(Track::Fit(limit));
        }
    }
    breadth(item, raw).map(Track::Breadth)
}

fn breadth<'a>(
    item: &ComponentValue<'_>,
    raw: &impl Fn(&ComponentValue<'_>) -> Option<&'a str>,
) -> Result<Breadth<'a>, Issue> {
    Ok(match item {
        ComponentValue::Number(number) if number.value == 0.0 => {
            Breadth::Fixed(Length::Points("0"))
        }
        ComponentValue::Percentage(percent) if percent.value.value >= 0.0 => {
            if percent.value.value > 10_000.0 {
                return Err(Issue::Limit);
            }
            Breadth::Fixed(Length::Percent(percent.value.value / 100.0))
        }
        ComponentValue::Dimension(dimension) if dimension.value.value >= 0.0 => {
            if dimension.unit.name.eq_ignore_ascii_case("fr") {
                if dimension.value.value > 100.0 {
                    return Err(Issue::Limit);
                }
                Breadth::Flex(dimension.value.value)
            } else if dimension.kind == DimensionKind::Length {
                Breadth::Fixed(Length::Points(raw(item).ok_or(Issue::Invalid)?))
            } else {
                return Err(Issue::Invalid);
            }
        }
        ComponentValue::InterpolableIdent(name) => {
            match ident(name)
                .ok_or(Issue::Invalid)?
                .to_ascii_lowercase()
                .as_str()
            {
                "auto" => Breadth::Auto,
                "min-content" => Breadth::MinContent,
                "max-content" => Breadth::MaxContent,
                "subgrid" | "masonry" => return Err(Issue::Unsupported),
                _ => return Err(Issue::Invalid),
            }
        }
        ComponentValue::BracketBlock(_) => return Err(Issue::Unsupported),
        ComponentValue::Function(_) => {
            let value = raw(item).ok_or(Issue::Invalid)?;
            if !calc::is_math(value) {
                return Err(Issue::Invalid);
            }
            if !calc::valid(value, true) {
                return Err(Issue::Invalid);
            }
            Breadth::Fixed(Length::Calc(value))
        }
        _ => return Err(Issue::Invalid),
    })
}

pub(in crate::epub) fn resolve(
    source: &str,
    font: f32,
    viewport: [f32; 2],
    lengths: &mut calc::CalculationTable,
) -> Result<Vec<GridTemplateComponent<String>>, EngineError> {
    if source.is_empty()
        || ["none", "initial", "unset", "revert"]
            .iter()
            .any(|keyword| source.eq_ignore_ascii_case(keyword))
    {
        return Ok(Vec::new());
    }
    if source.eq_ignore_ascii_case("revert-layer") {
        return Err(unsupported("EPUB grid cascade layers"));
    }
    let declaration = format!("value:{source}");
    let syntax = Parser::new(&declaration, Syntax::Css)
        .parse::<Declaration>()
        .map_err(|_| invalid("invalid EPUB grid track expression"))?;
    let tracks = parse(source, &syntax.value).map_err(|issue| match issue {
        Issue::Invalid => invalid("invalid EPUB grid track expression"),
        Issue::Unsupported => unsupported("unsupported EPUB grid track expression"),
        Issue::Limit => limited("EPUB grid track expression exceeds its limit"),
    })?;
    tracks
        .into_iter()
        .map(|track| {
            let mut length = |length| -> Result<LengthPercentage, EngineError> {
                Ok(match length {
                    Length::Points(raw) => {
                        let points = points(&raw.to_ascii_lowercase(), font, viewport)
                            .ok_or_else(|| unsupported("unsupported EPUB grid track unit"))?;
                        LengthPercentage::length(points)
                    }
                    Length::Percent(factor) => LengthPercentage::percent(factor),
                    Length::Calc(raw) => match calc::resolve(raw, font, viewport, false)? {
                        calc::Resolved::Points(points) => LengthPercentage::length(points),
                        calc::Resolved::Percent(percent) => {
                            LengthPercentage::percent(percent / 100.0)
                        }
                        calc::Resolved::Deferred(length) => {
                            LengthPercentage::calc(lengths.register(length)?)
                        }
                    },
                })
            };
            let mut breadth = |item| -> Result<TrackSizingFunction, EngineError> {
                Ok(match item {
                    Breadth::Fixed(value) => length(value)?.into(),
                    Breadth::Auto => auto(),
                    Breadth::MinContent => min_content(),
                    Breadth::MaxContent => max_content(),
                    Breadth::Flex(factor) => fr(factor),
                })
            };
            Ok(match track {
                Track::Breadth(item) => breadth(item)?,
                Track::MinMax(minimum, maximum) => TrackSizingFunction {
                    min: breadth(minimum)?.min,
                    max: breadth(maximum)?.max,
                },
                Track::Fit(limit) => fit_content(length(limit)?),
            }
            .into())
        })
        .collect()
}
