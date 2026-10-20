//! Bounded, factual notices about extraction projections; never a parser-error substitute.
use serde_json::{json, Value};

const MAX_CODES: usize = 100;
const MAX_SOURCES: usize = 100;

#[derive(Default)]
pub(crate) struct Diagnostics {
    entries: Vec<Entry>,
    sources: usize,
}

struct Entry {
    code: &'static str,
    severity: &'static str,
    message: &'static str,
    count: usize,
    sources: Vec<Value>,
}

impl Diagnostics {
    pub(crate) fn record(
        &mut self,
        code: &'static str,
        severity: &'static str,
        message: &'static str,
        source: Value,
    ) {
        let index = self.entries.iter().position(|entry| entry.code == code);
        let index = match index {
            Some(index) => index,
            None if self.entries.len() < MAX_CODES => {
                self.entries.push(Entry {
                    code,
                    severity,
                    message,
                    count: 0,
                    sources: Vec::new(),
                });
                self.entries.len() - 1
            }
            None => return,
        };
        let entry = &mut self.entries[index];
        entry.count = entry.count.saturating_add(1);
        if self.sources < MAX_SOURCES {
            entry.sources.push(source);
            self.sources += 1;
        }
    }

    pub(crate) fn into_json(self) -> Value {
        Value::Array(
            self.entries
                .into_iter()
                .map(|entry| {
                    json!({
                        "code": entry.code, "operation": "extract", "severity": entry.severity,
                        "message": entry.message, "count": entry.count, "sources": entry.sources,
                    })
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_source_examples_without_losing_occurrence_counts() {
        let mut diagnostics = Diagnostics::default();
        for index in 0..150 {
            diagnostics.record(
                "chart_data_omitted",
                "warning",
                "Chart data omitted",
                json!({"index": index}),
            );
        }
        diagnostics.record(
            "equation_linearized",
            "info",
            "Linear text",
            json!({"index": 151}),
        );
        let value = diagnostics.into_json();
        assert_eq!(value[0]["count"], 150);
        assert_eq!(value[0]["sources"].as_array().unwrap().len(), 100);
        assert_eq!(value[1]["count"], 1);
        assert_eq!(value[1]["sources"].as_array().unwrap().len(), 0);
    }
}
