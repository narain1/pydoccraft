//! Expand grid shorthands before the property cascade chooses their winners.
use raffia::{
    ast::{ComponentValue, Declaration, Delimiter, DelimiterKind},
    Parser, Spanned, Syntax,
};

use super::{function, keyword};
use crate::epub::css::{css_wide, valid_computed};

const PARTS: [&str; 6] = [
    "grid-template-rows",
    "grid-template-columns",
    "grid-template-areas",
    "grid-auto-rows",
    "grid-auto-columns",
    "grid-auto-flow",
];

pub(in crate::epub::css) fn template_parts(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "grid-template" => Some(&PARTS[..3]),
        "grid" => Some(&PARTS),
        _ => None,
    }
}

pub(in crate::epub::css) fn expand_template(value: &str, keys: &[&str]) -> Vec<(String, String)> {
    let values = if css_wide(value) {
        Some(std::array::from_fn(|_| value.to_owned()))
    } else if value.eq_ignore_ascii_case("none") {
        Some(defaults())
    } else {
        expand(value, keys.len() == PARTS.len())
    };
    values.map_or_else(Vec::new, |values| {
        keys.iter()
            .zip(values)
            .map(|(name, value)| ((*name).to_owned(), value))
            .collect()
    })
}

fn defaults() -> [String; 6] {
    ["none", "none", "none", "auto", "auto", "row"].map(str::to_owned)
}

fn expand(value: &str, implicit: bool) -> Option<[String; 6]> {
    let declaration = format!("value:{value}");
    let parsed = Parser::new(&declaration, Syntax::Css)
        .parse::<Declaration>()
        .ok()?;
    if parsed.span.end != declaration.len() || parsed.important.is_some() {
        return None;
    }
    let mut sides = parsed.value.split(|item| {
        matches!(
            item,
            ComponentValue::Delimiter(Delimiter {
                kind: DelimiterKind::Solidus,
                ..
            })
        )
    });
    let rows = sides.next()?;
    let columns = sides.next();
    if rows.is_empty() || columns.is_some_and(|items| items.is_empty()) || sides.next().is_some() {
        return None;
    }
    let mut values = defaults();
    if rows
        .iter()
        .any(|item| matches!(item, ComponentValue::InterpolableStr(_)))
    {
        let (sizes, areas) = area_rows(&declaration, rows)?;
        values[0] = sizes;
        values[2] = areas;
        if let Some(columns) = columns {
            if columns.iter().any(|item| !is_area_track_item(item)) {
                return None;
            }
            values[1] = raw(&declaration, columns)?.to_owned();
        }
    } else if let Some(columns) = columns {
        match (auto_tracks_prefix(rows), auto_tracks_prefix(columns)) {
            (Some((dense, auto)), None) if implicit => {
                values[1] = raw(&declaration, columns)?.to_owned();
                values[3] = raw(&declaration, auto).unwrap_or("auto").to_owned();
                values[5] = if dense { "row dense" } else { "row" }.to_owned();
            }
            (None, Some((dense, auto))) if implicit => {
                values[0] = raw(&declaration, rows)?.to_owned();
                values[4] = raw(&declaration, auto).unwrap_or("auto").to_owned();
                values[5] = if dense { "column dense" } else { "column" }.to_owned();
            }
            (None, None) => {
                values[0] = raw(&declaration, rows)?.to_owned();
                values[1] = raw(&declaration, columns)?.to_owned();
            }
            _ => return None,
        }
    } else {
        return None;
    }
    PARTS
        .iter()
        .zip(&values)
        .all(|(name, value)| {
            // A CSS-wide keyword can only apply to the whole shorthand.
            !css_wide(value) && valid_computed(name, value)
        })
        .then_some(values)
}

fn raw<'a>(source: &'a str, items: &[ComponentValue<'_>]) -> Option<&'a str> {
    source.get(items.first()?.span().start..items.last()?.span().end)
}

fn auto_tracks_prefix<'a, 's>(
    items: &'a [ComponentValue<'s>],
) -> Option<(bool, &'a [ComponentValue<'s>])> {
    match items {
        [first, second, tail @ ..]
            if (keyword(first, "auto-flow") && keyword(second, "dense"))
                || (keyword(first, "dense") && keyword(second, "auto-flow")) =>
        {
            Some((true, tail))
        }
        [first, tail @ ..] if keyword(first, "auto-flow") => Some((false, tail)),
        _ => None,
    }
}

fn is_area_track_item(item: &ComponentValue<'_>) -> bool {
    !["none", "subgrid", "masonry", "auto-flow", "dense"]
        .iter()
        .any(|name| keyword(item, name))
        && !function(item).is_some_and(|(name, _)| name.eq_ignore_ascii_case("repeat"))
}

fn area_rows(source: &str, items: &[ComponentValue<'_>]) -> Option<(String, String)> {
    let mut areas = Vec::new();
    let mut tracks = Vec::<String>::new();
    let mut needs_size = false;
    let mut adjacent_names = 0;
    for item in items {
        let span = item.span();
        let value = source.get(span.start..span.end)?;
        match item {
            ComponentValue::InterpolableStr(_) => {
                if needs_size {
                    tracks.push("auto".into());
                }
                areas.push(value);
                needs_size = true;
                adjacent_names = 0;
            }
            ComponentValue::BracketBlock(_) => {
                if needs_size {
                    tracks.push("auto".into());
                    needs_size = false;
                }
                adjacent_names += 1;
                match adjacent_names {
                    1 => tracks.push(value.to_owned()),
                    2 if !areas.is_empty() => {
                        // A preceding row's closing names and the next row's
                        // opening names describe one line in the longhand.
                        let names = tracks.last_mut()?;
                        names.pop()?;
                        names.push(' ');
                        names.push_str(value.strip_prefix('[')?);
                    }
                    _ => return None,
                }
            }
            _ if needs_size && is_area_track_item(item) => {
                tracks.push(value.to_owned());
                needs_size = false;
            }
            _ => return None,
        }
    }
    if adjacent_names > 1 {
        return None;
    }
    if needs_size {
        tracks.push("auto".into());
    }
    Some((tracks.join(" "), areas.join(" ")))
}
