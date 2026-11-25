//! Evaluate the bounded TTML boolean profile for ordinary subtitle playback.
use std::collections::HashSet;

use roxmltree::{Node, NodeId};

use super::{
    invalid, limited,
    ttml_clock::{Clock, Timing},
    ttml_styles::{self, TT},
    unsupported, EngineError,
};

const MAX_EXPRESSION_BYTES: usize = 1024;
const MAX_EXPRESSION_DEPTH: usize = 16;

pub(super) fn read(root: Node<'_, '_>) -> Result<HashSet<NodeId>, EngineError> {
    let mut excluded = HashSet::new();
    let mut clock = None;
    for node in root.descendants().filter(Node::is_element) {
        let Some(condition) = node.attribute("condition") else {
            continue;
        };
        if node.tag_name().namespace() != Some(TT)
            || node
                .ancestors()
                .skip(1)
                .any(|ancestor| excluded.contains(&ancestor.id()))
        {
            continue;
        }
        let is_set = match node.tag_name().name() {
            "body" | "div" | "p" | "span" | "br" => false,
            // The shared region validator retains its existing diagnostics.
            "layout" | "region" => continue,
            "set" => {
                validate_set(node)?;
                true
            }
            _ => return Err(unsupported("unsupported TTML conditional target")),
        };
        let included = evaluate(condition)?;
        if included && !is_set {
            continue;
        }
        if !is_set && !node.has_attribute("dur") && !node.has_attribute("end") {
            return Err(unsupported(
                "excluded TTML content requires an explicit end or duration",
            ));
        }
        let clock = match clock {
            Some(clock) => clock,
            None => *clock.insert(Clock::new(root)?),
        };
        // Validate timing regardless of whether the animation or content is shown.
        Timing::new(node, 0, None, clock)?;
        if !included {
            excluded.insert(node.id());
        }
    }
    Ok(excluded)
}

fn validate_set(node: Node<'_, '_>) -> Result<(), EngineError> {
    let parent = node
        .parent_element()
        .filter(|parent| parent.tag_name().namespace() == Some(TT))
        .ok_or_else(|| unsupported("unsupported TTML conditional set target"))?;
    match parent.tag_name().name() {
        "region" => Ok(()),
        "body" | "div" | "p" | "span" | "br" => {
            if parent.attribute("timeContainer") == Some("seq") {
                return Err(unsupported(
                    "inline TTML set in a sequential time container",
                ));
            }
            ttml_styles::content_set(node).map(|_| ())
        }
        _ => Err(unsupported("unsupported TTML conditional set target")),
    }
}

fn evaluate(expression: &str) -> Result<bool, EngineError> {
    if expression.len() > MAX_EXPRESSION_BYTES {
        return Err(limited("TTML condition exceeds its expression byte budget"));
    }
    let mut parser = Parser { rest: expression };
    let value = parser.expression(1, 0)?;
    if !parser.rest.trim_ascii().is_empty() {
        return Err(malformed());
    }
    Ok(value)
}

fn malformed() -> EngineError {
    invalid("invalid TTML condition expression")
}

struct Parser<'a> {
    rest: &'a str,
}

impl Parser<'_> {
    fn expression(&mut self, minimum: usize, depth: usize) -> Result<bool, EngineError> {
        let mut value = self.atom(depth)?;
        loop {
            let token = self.rest.trim_ascii_start();
            let Some((operator, precedence)) = [("||", 1), ("&&", 2), ("==", 3), ("!=", 3)]
                .into_iter()
                .find(|(name, _)| token.starts_with(name))
            else {
                return Ok(value);
            };
            if precedence < minimum {
                return Ok(value);
            }
            self.rest = &token[operator.len()..];
            let right = self.expression(precedence + 1, depth + 1)?;
            value = match operator {
                "||" => value || right,
                "&&" => value && right,
                "==" => value == right,
                _ => value != right,
            };
        }
    }

    fn atom(&mut self, depth: usize) -> Result<bool, EngineError> {
        if depth >= MAX_EXPRESSION_DEPTH {
            return Err(limited(
                "TTML condition exceeds its expression nesting budget",
            ));
        }
        if self.take("!") {
            return self.atom(depth + 1).map(|value| !value);
        }
        if self.take("(") {
            let value = self.expression(1, depth + 1)?;
            return if self.take(")") {
                Ok(value)
            } else {
                Err(malformed())
            };
        }
        if self.take("true") {
            return Ok(true);
        }
        if self.take("false") {
            return Ok(false);
        }
        if self.take("parameter(") {
            return match self.argument()? {
                "forced" => Ok(false), // The EPUB APIs use ordinary subtitle playback.
                "mediaAspectRatio" | "mediaLanguage" | "userLanguage" => Err(unsupported(
                    "TTML condition parameter has no EPUB playback binding",
                )),
                _ => Err(malformed()),
            };
        }
        if self.take("media(") || self.take("supports(") {
            self.argument()?;
            return Err(unsupported("unsupported TTML condition context function"));
        }
        Err(malformed())
    }

    fn argument(&mut self) -> Result<&str, EngineError> {
        let rest = self.rest.trim_ascii_start();
        let quote = *rest.as_bytes().first().ok_or_else(malformed)?;
        if !matches!(quote, b'\'' | b'"') {
            return Err(malformed());
        }
        let (value, after) = rest[1..]
            .split_once(char::from(quote))
            .ok_or_else(malformed)?;
        if value.contains('\\') {
            return Err(unsupported("escaped TTML condition argument"));
        }
        self.rest = after;
        if !self.take(")") {
            return Err(malformed());
        }
        Ok(value)
    }

    fn take(&mut self, token: &str) -> bool {
        let Some(rest) = self.rest.trim_ascii_start().strip_prefix(token) else {
            return false;
        };
        self.rest = rest;
        true
    }
}
