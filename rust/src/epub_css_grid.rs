//! Parse the EPUB grid track profile and project it into the native layout tree.
use std::collections::BTreeMap;

use raffia::{
    ast::{ComponentValue, Declaration, Delimiter, DelimiterKind, DimensionKind, FunctionName},
    Parser, Spanned, Syntax,
};
use taffy::{
    geometry::Line,
    style::{
        GridAutoFlow, GridPlacement, GridTemplateArea, GridTemplateAreas, GridTemplateComponent,
        GridTemplateRepetition, LengthPercentage, RepetitionCount, TrackSizingFunction,
    },
    style_helpers::{auto, fit_content, fr, max_content, min_content},
};

use super::{calc, ident, points};
use crate::{
    epub::{invalid, limited, unsupported},
    EngineError,
};

pub(in crate::epub) const MAX_TRACKS: usize = 32;
pub(in crate::epub) const MAX_LINE_NAMES: usize = 128;
const MAX_LINE_NAME_BYTES: usize = 128;
const MAX_PLACEMENT: i16 = 64;

#[path = "epub_css_grid_template.rs"]
mod shorthand;
pub(super) use shorthand::{expand_template, template_parts};

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

enum TrackPart<'a> {
    Single(Track<'a>),
    Automatic(RepetitionCount, Template<Track<'a>>),
}

impl Track<'_> {
    fn has_fixed_breadth(self) -> bool {
        matches!(
            self,
            Self::Breadth(Breadth::Fixed(_))
                | Self::MinMax(Breadth::Fixed(_), _)
                | Self::MinMax(_, Breadth::Fixed(_))
        )
    }
}

enum Issue {
    Invalid,
    Unsupported,
    Limit,
}

#[derive(Clone, Copy)]
pub(in crate::epub) struct NameRepeat {
    pub start: usize,
    pub length: usize,
}

pub(in crate::epub) struct Template<T> {
    pub tracks: Vec<T>,
    pub names: Vec<Vec<String>>,
    pub subgrid: bool,
    pub name_repeat: Option<NameRepeat>,
}

impl<T> Default for Template<T> {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            names: Vec::new(),
            subgrid: false,
            name_repeat: None,
        }
    }
}

impl<T> Template<T> {
    fn with_first_line() -> Self {
        Self {
            tracks: Vec::new(),
            names: vec![Vec::new()],
            subgrid: false,
            name_repeat: None,
        }
    }
}

pub(super) fn valid(value: &str, components: &[ComponentValue<'_>]) -> bool {
    !matches!(parse(value, components), Err(Issue::Invalid))
}

pub(super) fn valid_auto_tracks(value: &str, components: &[ComponentValue<'_>]) -> bool {
    !components.iter().any(|item| {
        matches!(item, ComponentValue::BracketBlock(_))
            || function(item).is_some_and(|(name, _)| name.eq_ignore_ascii_case("repeat"))
            || keyword(item, "subgrid")
            || keyword(item, "masonry")
            || keyword(item, "none")
    }) && valid(value, components)
}

fn keyword(item: &ComponentValue<'_>, keyword: &str) -> bool {
    matches!(item, ComponentValue::InterpolableIdent(name)
        if ident(name).is_some_and(|name| name.eq_ignore_ascii_case(keyword)))
}

pub(super) fn flow(components: &[ComponentValue<'_>]) -> Option<GridAutoFlow> {
    let axis = |item| {
        if keyword(item, "row") {
            Some(GridAutoFlow::Row)
        } else if keyword(item, "column") {
            Some(GridAutoFlow::Column)
        } else {
            None
        }
    };
    let dense = |flow| match flow {
        GridAutoFlow::Column => GridAutoFlow::ColumnDense,
        _ => GridAutoFlow::RowDense,
    };
    match components {
        [item] if keyword(item, "dense") => Some(GridAutoFlow::RowDense),
        [item] => axis(item),
        [first, last] if keyword(first, "dense") => axis(last).map(dense),
        [first, last] if keyword(last, "dense") => axis(first).map(dense),
        _ => None,
    }
}

pub(in crate::epub) fn resolve_flow(
    source: &str,
    inherited: impl FnOnce() -> GridAutoFlow,
) -> Result<GridAutoFlow, EngineError> {
    if source.eq_ignore_ascii_case("inherit") {
        return Ok(inherited());
    }
    if source.eq_ignore_ascii_case("revert-layer") {
        return Err(unsupported("EPUB grid cascade layers"));
    }
    if source.is_empty() || super::css_wide(source) {
        return Ok(GridAutoFlow::Row);
    }
    let declaration = format!("value:{source}");
    let syntax = Parser::new(&declaration, Syntax::Css)
        .parse::<Declaration>()
        .map_err(|_| invalid("invalid EPUB grid auto flow"))?;
    flow(&syntax.value).ok_or_else(|| invalid("invalid EPUB grid auto flow"))
}

fn parse<'a>(
    value: &'a str,
    components: &[ComponentValue<'_>],
) -> Result<Template<TrackPart<'a>>, Issue> {
    if value.eq_ignore_ascii_case("none") {
        return Ok(Template::default());
    }
    if let [first, lines @ ..] = components {
        if keyword(first, "subgrid") {
            return parse_subgrid(lines);
        }
    }
    let mut template = Template::with_first_line();
    let offset = components.first().ok_or(Issue::Invalid)?.span().start;
    let raw = |item: &ComponentValue<'_>| {
        let span = item.span();
        value.get(span.start.checked_sub(offset)?..span.end.checked_sub(offset)?)
    };
    let mut names = 0;
    let mut automatic_tracks: usize = 0;
    let mut previous_line = false;
    for item in components {
        check_adjacent_lines(item, &mut previous_line)?;
        if let Some((_, args)) =
            function(item).filter(|(name, _)| name.eq_ignore_ascii_case("repeat"))
        {
            let [count, delimiter, list @ ..] = args else {
                return Err(Issue::Invalid);
            };
            if !comma(delimiter) || list.is_empty() {
                return Err(Issue::Invalid);
            }
            let (count, automatic) = match count {
                ComponentValue::Number(count) => (repeat_count(count.raw)?, None),
                item if keyword(item, "auto-fit") => (1, Some(RepetitionCount::AutoFit)),
                item if keyword(item, "auto-fill") => (1, Some(RepetitionCount::AutoFill)),
                _ => return Err(Issue::Invalid),
            };
            let mut repeated = Template::with_first_line();
            let mut repeated_names = 0;
            let mut previous_line = false;
            for item in list {
                check_adjacent_lines(item, &mut previous_line)?;
                append_slot(&mut repeated, item, &raw, &mut repeated_names, |track| {
                    track
                })?;
            }
            let track_count = repeated.tracks.len();
            if track_count == 0 {
                return Err(Issue::Invalid);
            }
            let occupied = template.tracks.len() + automatic_tracks.saturating_sub(1);
            if count > (MAX_TRACKS - occupied) / track_count
                || repeated_names.saturating_mul(count) > MAX_LINE_NAMES - names
            {
                return Err(Issue::Limit);
            }
            names += repeated_names * count;
            if let Some(mode) = automatic {
                if automatic_tracks != 0
                    || !repeated
                        .tracks
                        .iter()
                        .all(|track| track.has_fixed_breadth())
                {
                    return Err(Issue::Invalid);
                }
                automatic_tracks = track_count;
                if repeated_names == 0 {
                    repeated.names.clear();
                }
                template.tracks.push(TrackPart::Automatic(mode, repeated));
                template.names.push(Vec::new());
            } else {
                for _ in 0..count {
                    template
                        .tracks
                        .extend(repeated.tracks.iter().copied().map(TrackPart::Single));
                    merge_names(template.names.last_mut().unwrap(), &repeated.names[0]);
                    template.names.extend_from_slice(&repeated.names[1..]);
                }
            }
        } else {
            append_slot(&mut template, item, &raw, &mut names, TrackPart::Single)?;
            if template.tracks.len() + automatic_tracks.saturating_sub(1) > MAX_TRACKS {
                return Err(Issue::Limit);
            }
        }
    }
    if template.tracks.is_empty()
        || (automatic_tracks != 0
            && template
                .tracks
                .iter()
                .any(|part| matches!(part, TrackPart::Single(track) if !track.has_fixed_breadth())))
    {
        return Err(Issue::Invalid);
    }
    if names == 0 {
        template.names.clear();
    }
    Ok(template)
}

fn parse_subgrid<'a>(items: &[ComponentValue<'_>]) -> Result<Template<TrackPart<'a>>, Issue> {
    let mut template = Template {
        subgrid: true,
        ..Template::default()
    };
    let mut total = 0;
    for item in items {
        let (lists, count) = if let Some((_, args)) =
            function(item).filter(|(name, _)| name.eq_ignore_ascii_case("repeat"))
        {
            let [count, delimiter, lists @ ..] = args else {
                return Err(Issue::Invalid);
            };
            if !comma(delimiter) || lists.is_empty() {
                return Err(Issue::Invalid);
            }
            let count = match count {
                ComponentValue::Number(number) => Some(repeat_count(number.raw)?),
                item if keyword(item, "auto-fill") && template.name_repeat.is_none() => None,
                _ => return Err(Issue::Invalid),
            };
            (lists, count)
        } else {
            (std::slice::from_ref(item), Some(1))
        };

        let repeated = count.unwrap_or(1);
        let slots = lists.len().checked_mul(repeated).ok_or(Issue::Limit)?;
        if slots > MAX_TRACKS + 1 - template.names.len() {
            return Err(Issue::Limit);
        }
        let mut group = Vec::with_capacity(lists.len());
        for list in lists {
            let ComponentValue::BracketBlock(block) = list else {
                return Err(Issue::Invalid);
            };
            let expanded = block
                .value
                .len()
                .checked_mul(repeated)
                .ok_or(Issue::Limit)?;
            if expanded > MAX_LINE_NAMES - total {
                return Err(Issue::Limit);
            }
            total += expanded;
            group.push(
                block
                    .value
                    .iter()
                    .map(|name| line_name(name).map(str::to_owned))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }

        if count.is_none() {
            template.name_repeat = Some(NameRepeat {
                start: template.names.len(),
                length: group.len(),
            });
        }
        for _ in 1..repeated {
            template.names.extend(group.iter().cloned());
        }
        template.names.extend(group);
    }
    Ok(template)
}

fn repeat_count(source: &str) -> Result<usize, Issue> {
    let digits = source.strip_prefix('+').unwrap_or(source);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Issue::Invalid);
    }
    let count = digits.parse::<usize>().map_err(|_| Issue::Limit)?;
    if count == 0 {
        return Err(Issue::Invalid);
    }
    Ok(count)
}

pub(in crate::epub) fn subgrid_placement(
    mut line: Line<GridPlacement<String>>,
    names: usize,
    repeat: Option<NameRepeat>,
) -> Line<GridPlacement<String>> {
    let fixed = names - repeat.map_or(0, |repeat| repeat.length);
    let span = GridPlacement::Span(fixed.saturating_sub(1).max(1) as u16);
    match (&line.start, &line.end) {
        (
            GridPlacement::Auto | GridPlacement::Line(_) | GridPlacement::NamedLine(..),
            GridPlacement::Auto,
        ) => line.end = span,
        (GridPlacement::Auto, GridPlacement::Line(_) | GridPlacement::NamedLine(..)) => {
            line.start = span
        }
        _ => {}
    }
    line
}

fn check_adjacent_lines(item: &ComponentValue<'_>, previous: &mut bool) -> Result<(), Issue> {
    let line = matches!(item, ComponentValue::BracketBlock(_));
    if std::mem::replace(previous, line) && line {
        return Err(Issue::Invalid);
    }
    Ok(())
}

fn merge_names(line: &mut Vec<String>, more: &[String]) {
    for name in more {
        if !line.contains(name) {
            line.push(name.clone());
        }
    }
}

fn append_slot<'a, T>(
    template: &mut Template<T>,
    item: &ComponentValue<'_>,
    raw: &impl Fn(&ComponentValue<'_>) -> Option<&'a str>,
    names: &mut usize,
    convert: impl FnOnce(Track<'a>) -> T,
) -> Result<(), Issue> {
    if let ComponentValue::BracketBlock(block) = item {
        let line = template.names.last_mut().unwrap();
        for item in &block.value {
            let name = line_name(item)?;
            *names += 1;
            if *names > MAX_LINE_NAMES {
                return Err(Issue::Limit);
            }
            if !line.iter().any(|item| item == name) {
                line.push(name.to_owned());
            }
        }
    } else {
        if template.tracks.len() == MAX_TRACKS {
            return Err(Issue::Limit);
        }
        template.tracks.push(convert(track(item, raw)?));
        template.names.push(Vec::new());
    }
    Ok(())
}

fn line_name<'a>(item: &'a ComponentValue<'_>) -> Result<&'a str, Issue> {
    let ComponentValue::InterpolableIdent(name) = item else {
        return Err(Issue::Invalid);
    };
    let name = ident(name).ok_or(Issue::Invalid)?;
    if super::css_wide(name)
        || ["span", "auto", "default"]
            .iter()
            .any(|key| name.eq_ignore_ascii_case(key))
    {
        return Err(Issue::Invalid);
    }
    if name.len() > MAX_LINE_NAME_BYTES {
        return Err(Issue::Limit);
    }
    Ok(name)
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

pub(super) fn placement_parts(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "grid-column" => &["grid-column-start", "grid-column-end"],
        "grid-row" => &["grid-row-start", "grid-row-end"],
        "grid-area" => &[
            "grid-row-start",
            "grid-column-start",
            "grid-row-end",
            "grid-column-end",
        ],
        _ => return None,
    })
}

pub(super) fn is_placement(name: &str) -> bool {
    matches!(
        name,
        "grid-column-start" | "grid-column-end" | "grid-row-start" | "grid-row-end"
    )
}

pub(super) fn valid_placement(items: &[ComponentValue<'_>]) -> bool {
    !matches!(placement(items), Err(Issue::Invalid))
}

fn placement(items: &[ComponentValue<'_>]) -> Result<GridPlacement<String>, Issue> {
    let mut span = false;
    let mut number = None;
    let mut name = None;
    for item in items {
        match item {
            ComponentValue::InterpolableIdent(key)
                if ident(key).is_some_and(|key| key.eq_ignore_ascii_case("auto")) =>
            {
                return if items.len() == 1 {
                    Ok(GridPlacement::Auto)
                } else {
                    Err(Issue::Invalid)
                };
            }
            ComponentValue::InterpolableIdent(key)
                if ident(key).is_some_and(|key| key.eq_ignore_ascii_case("span")) && !span =>
            {
                span = true;
            }
            ComponentValue::Number(item) if number.is_none() => {
                let digits = item.raw.trim_start_matches(['+', '-']);
                if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(Issue::Invalid);
                }
                let index = item.raw.parse::<i16>().map_err(|_| Issue::Limit)?;
                if index == 0 {
                    return Err(Issue::Invalid);
                }
                if index.unsigned_abs() > MAX_PLACEMENT as u16 {
                    return Err(Issue::Limit);
                }
                number = Some(index);
            }
            ComponentValue::InterpolableIdent(_) if name.is_none() => {
                name = Some(line_name(item)?);
            }
            _ => return Err(Issue::Invalid),
        }
    }
    Ok(match (span, number, name) {
        (false, Some(index), None) => taffy::style_helpers::line(index),
        (false, number, Some(name)) => {
            GridPlacement::NamedLine(name.to_owned(), number.unwrap_or(0))
        }
        (true, Some(index), None) if index > 0 => GridPlacement::Span(index as u16),
        (true, number, Some(name)) if number.unwrap_or(1) > 0 => {
            GridPlacement::NamedSpan(name.to_owned(), number.unwrap_or(1) as u16)
        }
        _ => return Err(Issue::Invalid),
    })
}

pub(super) fn expand_placement(name: &str, value: &str, keys: &[&str]) -> Vec<(String, String)> {
    if super::css_wide(value) {
        return keys
            .iter()
            .map(|key| ((*key).to_owned(), value.to_owned()))
            .collect();
    }
    let declaration = format!("value:{value}");
    let Ok(syntax) = Parser::new(&declaration, Syntax::Css).parse::<Declaration>() else {
        return Vec::new();
    };
    let pieces: Option<Vec<_>> = syntax
        .value
        .split(|item| {
            matches!(
                item,
                ComponentValue::Delimiter(Delimiter {
                    kind: DelimiterKind::Solidus,
                    ..
                })
            )
        })
        .map(|items| {
            if !valid_placement(items) {
                return None;
            }
            let component = &declaration[items.first()?.span().start..items.last()?.span().end];
            let bare = matches!(items, [item] if !matches!(line_name(item), Err(Issue::Invalid)));
            Some((component, bare))
        })
        .collect();
    let Some(pieces) = pieces.filter(|pieces| !pieces.is_empty() && pieces.len() <= keys.len())
    else {
        return Vec::new();
    };
    let fallback = |(value, bare)| if bare { (value, true) } else { ("auto", false) };
    let first = pieces[0];
    let second = pieces.get(1).copied().unwrap_or_else(|| fallback(first));
    let values = if name == "grid-area" {
        [
            first,
            second,
            pieces.get(2).copied().unwrap_or_else(|| fallback(first)),
            pieces.get(3).copied().unwrap_or_else(|| fallback(second)),
        ]
    } else {
        [first, second, ("auto", false), ("auto", false)]
    };
    keys.iter()
        .zip(values)
        .map(|(key, (value, _))| ((*key).to_owned(), value.to_owned()))
        .collect()
}

pub(in crate::epub) fn resolve_placement(
    source: &str,
    inherited: impl FnOnce() -> GridPlacement<String>,
) -> Result<GridPlacement<String>, EngineError> {
    if source.eq_ignore_ascii_case("inherit") {
        return Ok(inherited());
    }
    if source.eq_ignore_ascii_case("revert-layer") {
        return Err(unsupported("EPUB grid cascade layers"));
    }
    if source.is_empty() || super::css_wide(source) {
        return Ok(GridPlacement::Auto);
    }
    let declaration = format!("value:{source}");
    let syntax = Parser::new(&declaration, Syntax::Css)
        .parse::<Declaration>()
        .map_err(|_| invalid("invalid EPUB grid placement"))?;
    placement(&syntax.value).map_err(|issue| match issue {
        Issue::Invalid => invalid("invalid EPUB grid placement"),
        Issue::Unsupported => unsupported("unsupported EPUB grid placement"),
        Issue::Limit => limited("EPUB grid placement exceeds its limit"),
    })
}

pub(super) fn valid_areas(value: &str, items: &[ComponentValue<'_>]) -> bool {
    value.eq_ignore_ascii_case("none") || !matches!(areas(items), Err(Issue::Invalid))
}

fn areas(items: &[ComponentValue<'_>]) -> Result<GridTemplateAreas<String>, Issue> {
    if items.len() > MAX_TRACKS {
        return Err(Issue::Limit);
    }
    let mut regions = BTreeMap::<&str, (GridTemplateArea<String>, u16)>::new();
    let mut column_count = 0;
    for (row, item) in items.iter().enumerate() {
        let ComponentValue::InterpolableStr(source) = item else {
            return Err(Issue::Invalid);
        };
        let mut rest = super::string_value(source).ok_or(Issue::Invalid)?;
        let mut column = 0;
        let row = row as u16 + 1;
        while let Some(first) = rest.chars().next() {
            if matches!(first, ' ' | '\t' | '\r' | '\n' | '\x0c') {
                rest = &rest[1..];
                continue;
            }
            let named = first != '.';
            let ident_code =
                |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_') || !c.is_ascii();
            if named && !ident_code(first) {
                return Err(Issue::Invalid);
            }
            let end = rest
                .find(|c| if named { !ident_code(c) } else { c != '.' })
                .unwrap_or(rest.len());
            let (cell, tail) = rest.split_at(end);
            rest = tail;
            column += 1;
            if column > MAX_TRACKS as u16 || named && cell.len() > MAX_LINE_NAME_BYTES {
                return Err(Issue::Limit);
            }
            if named {
                let (region, occupied) = regions.entry(cell).or_insert_with(|| {
                    (
                        GridTemplateArea {
                            name: cell.to_owned(),
                            row_start: row,
                            row_end: row + 1,
                            column_start: column,
                            column_end: column + 1,
                        },
                        0,
                    )
                });
                *occupied += 1;
                region.row_end = row + 1;
                region.column_start = region.column_start.min(column);
                region.column_end = region.column_end.max(column + 1);
            }
        }
        if column == 0 || column_count != 0 && column != column_count {
            return Err(Issue::Invalid);
        }
        column_count = column;
    }
    if column_count == 0 {
        return Err(Issue::Invalid);
    }
    let areas = regions
        .into_values()
        .map(|(region, occupied)| {
            let expected =
                (region.row_end - region.row_start) * (region.column_end - region.column_start);
            if expected != occupied {
                Err(Issue::Invalid)
            } else {
                Ok(region)
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(GridTemplateAreas {
        areas,
        row_count: items.len() as u16,
        column_count,
    })
}

pub(in crate::epub) fn resolve_areas(
    source: &str,
    inherited: impl FnOnce() -> Option<GridTemplateAreas<String>>,
) -> Result<Option<GridTemplateAreas<String>>, EngineError> {
    if source.eq_ignore_ascii_case("inherit") {
        return Ok(inherited());
    }
    if source.eq_ignore_ascii_case("revert-layer") {
        return Err(unsupported("EPUB grid cascade layers"));
    }
    if source.is_empty() || source.eq_ignore_ascii_case("none") || super::css_wide(source) {
        return Ok(None);
    }
    let declaration = format!("value:{source}");
    let syntax = Parser::new(&declaration, Syntax::Css)
        .parse::<Declaration>()
        .map_err(|_| invalid("invalid EPUB grid area template"))?;
    areas(&syntax.value).map(Some).map_err(|issue| match issue {
        Issue::Limit => limited("EPUB grid area template exceeds its limit"),
        _ => invalid("invalid EPUB grid area template"),
    })
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

pub(in crate::epub) fn resolve_auto_tracks(
    source: &str,
    font: f32,
    viewport: [f32; 2],
    lengths: &mut calc::CalculationTable,
) -> Result<Vec<TrackSizingFunction>, EngineError> {
    resolve(source, font, viewport, lengths)?
        .tracks
        .into_iter()
        .map(|part| match part {
            GridTemplateComponent::Single(track) => Ok(track),
            GridTemplateComponent::Repeat(_) => Err(invalid("invalid EPUB implicit grid tracks")),
        })
        .collect()
}

pub(in crate::epub) fn resolve(
    source: &str,
    font: f32,
    viewport: [f32; 2],
    lengths: &mut calc::CalculationTable,
) -> Result<Template<GridTemplateComponent<String>>, EngineError> {
    if source.is_empty()
        || ["none", "initial", "unset", "revert"]
            .iter()
            .any(|keyword| source.eq_ignore_ascii_case(keyword))
    {
        return Ok(Template::default());
    }
    if source.eq_ignore_ascii_case("revert-layer") {
        return Err(unsupported("EPUB grid cascade layers"));
    }
    let declaration = format!("value:{source}");
    let syntax = Parser::new(&declaration, Syntax::Css)
        .parse::<Declaration>()
        .map_err(|_| invalid("invalid EPUB grid track expression"))?;
    let Template {
        tracks,
        names,
        subgrid,
        name_repeat,
    } = parse(source, &syntax.value).map_err(|issue| match issue {
        Issue::Invalid => invalid("invalid EPUB grid track expression"),
        Issue::Unsupported => unsupported("unsupported EPUB grid track expression"),
        Issue::Limit => limited("EPUB grid track expression exceeds its limit"),
    })?;
    let mut resolve_track = |track: Track<'_>| -> Result<TrackSizingFunction, EngineError> {
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
                    calc::Resolved::Percent(percent) => LengthPercentage::percent(percent / 100.0),
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
        })
    };
    let tracks = tracks
        .into_iter()
        .map(|part| match part {
            TrackPart::Single(track) => Ok(GridTemplateComponent::Single(resolve_track(track)?)),
            TrackPart::Automatic(count, repeated) => {
                let tracks = repeated
                    .tracks
                    .into_iter()
                    .map(&mut resolve_track)
                    .collect::<Result<_, _>>()?;
                Ok(GridTemplateComponent::Repeat(GridTemplateRepetition {
                    count,
                    tracks,
                    line_names: repeated.names,
                }))
            }
        })
        .collect::<Result<_, EngineError>>()?;
    Ok(Template {
        tracks,
        names,
        subgrid,
        name_repeat,
    })
}
