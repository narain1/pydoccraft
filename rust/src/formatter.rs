//! Bounded formatting primitives for snapshots of mutable Python models.

pub fn join_parts(parts: Vec<String>, separator: &str, skip_empty: bool) -> String {
    parts
        .into_iter()
        .filter(|part| !skip_empty || !part.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_whitespace_for_python_strip() {
        assert_eq!(
            join_parts(
                vec!["\u{1c}a".into(), "".into(), "b\u{1f}".into()],
                "\n",
                true
            ),
            "\u{1c}a\nb\u{1f}"
        );
    }
}
