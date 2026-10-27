use crate::{LabResult, fail};
use savior::{Policy, Span, validate_text};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub id: String,
    pub family: String,
    pub category: String,
    pub split: String,
    pub text: String,
    pub labels: BTreeMap<String, Vec<[usize; 2]>>,
    pub provenance: String,
    pub source: String,
    pub license: String,
    pub reviewed: bool,
}
impl Example {
    pub fn spans(&self, policy: Policy) -> Vec<Span> {
        self.labels[policy.name()]
            .iter()
            .map(|s| Span {
                start: s[0],
                end: s[1],
                score: 1.0,
            })
            .collect()
    }
}
pub fn read(path: &Path) -> LabResult<Vec<Example>> {
    let data = std::fs::read_to_string(path).map_err(|_| "corpus_unavailable")?;
    let examples: Vec<Example> = data
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|_| "invalid_corpus".to_string()))
        .collect::<LabResult<_>>()?;
    validate(&examples)?;
    Ok(examples)
}
pub fn validate(examples: &[Example]) -> LabResult<()> {
    if examples.is_empty() {
        return fail("empty_corpus");
    }
    let mut ids = BTreeSet::new();
    let mut families = BTreeMap::new();
    let mut texts = BTreeSet::new();
    for e in examples {
        let safe = |s: &str| {
            !s.is_empty()
                && s.len() <= 100
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        };
        if !safe(&e.id)
            || !safe(&e.family)
            || !safe(&e.category)
            || !ids.insert(&e.id)
            || !texts.insert(&e.text)
            || !["train", "validation", "test"].contains(&e.split.as_str())
            || e.provenance != "synthetic_template"
                && e.provenance != "licensed"
                && e.provenance != "judge_proposal"
            || e.provenance == "judge_proposal" && !e.reviewed
            || e.source.trim().is_empty()
            || e.license.trim().is_empty()
            || e.labels.len() != 2
            || validate_text(&e.text).is_err()
        {
            return fail("invalid_corpus");
        }
        if let Some(previous) = families.insert(&e.family, &e.split)
            && previous != &e.split
        {
            return fail("split_leakage");
        }
        for policy in [Policy::OffensiveSpans, Policy::DirectedAbuse] {
            let spans = e.labels.get(policy.name()).ok_or("missing_policy_labels")?;
            let mut previous = 0;
            for &[start, end] in spans {
                if start < previous
                    || start >= end
                    || end > e.text.len()
                    || !e.text.is_char_boundary(start)
                    || !e.text.is_char_boundary(end)
                {
                    return fail("invalid_label_span");
                }
                previous = end;
            }
        }
    }
    Ok(())
}

pub fn bootstrap(path: &Path) -> LabResult<()> {
    let groups: [(&str, [&str; 4], &str, bool); 10] = [
        (
            "ordinary",
            [
                "Please inspect the implementation.",
                "Check the test result.",
                "Read the failing assertion.",
                "Find the source of the regression.",
            ],
            "",
            false,
        ),
        (
            "criticism",
            [
                "Your previous answer was wrong.",
                "That explanation is unhelpful.",
                "The change made the bug worse.",
                "You ignored the requested behavior.",
            ],
            "",
            false,
        ),
        (
            "profanity",
            [
                "This is fucking frustrating.",
                "The fucking build failed.",
                "That is fucking unexpected.",
                "The output is fucking confusing.",
            ],
            "fucking",
            false,
        ),
        (
            "directed",
            [
                "You idiot, investigate this.",
                "Stop guessing, idiot.",
                "Listen, idiot: inspect the result.",
                "An idiot wrote this implementation.",
            ],
            "idiot",
            true,
        ),
        (
            "phrase",
            [
                "You stupid idiot, inspect the failure.",
                "Listen, stupid idiot: read the code.",
                "Stop guessing, you stupid idiot.",
                "A stupid idiot wrote the test.",
            ],
            "stupid idiot",
            true,
        ),
        (
            "quoted",
            [
                "Explain the phrase \"you idiot\".",
                "Translate \"you idiot\" into French.",
                "The fixture contains \"you idiot\".",
                "Preserve the quotation \"you idiot\".",
            ],
            "idiot",
            true,
        ),
        (
            "code",
            [
                "Keep `idiot_name` unchanged.",
                "Inspect `/src/idiot.rs`.",
                "Explain `let idiot = 42;`.",
                "Preserve `https://example.com/idiot`.",
            ],
            "idiot",
            true,
        ),
        (
            "unicode",
            [
                "🙂 You idiot, inspect the failure.",
                "Café: you idiot, check the output.",
                "東京 — you idiot, read the assertion.",
                "Δ: you idiot, check the boundary.",
            ],
            "idiot",
            true,
        ),
        (
            "negation",
            [
                "You idiot, do not delete the test.",
                "You idiot, never disable validation.",
                "You idiot, do not change the assertion.",
                "You idiot, keep the retry limit at 42.",
            ],
            "idiot",
            true,
        ),
        (
            "window",
            [
                "You idiot, inspect the final token.",
                "Stop guessing, idiot: inspect the boundary.",
                "Listen, idiot, preserve the suffix.",
                "An idiot changed the final assertion.",
            ],
            "idiot",
            true,
        ),
    ];
    let tasks = [
        "Run cargo test.",
        "Preserve the public API.",
        "Keep the existing behavior.",
        "Check the error path.",
        "Retain all assertions.",
        "Explain the root cause.",
        "Preserve numeric constants.",
        "Check the cancellation path.",
    ];
    let mut examples = Vec::new();
    for (category_index, (category, templates, phrase, directed)) in groups.iter().enumerate() {
        for (family_index, template) in templates.iter().enumerate() {
            let split = match family_index {
                0..=1 => "train",
                2 => "validation",
                _ => "test",
            };
            let count = match family_index {
                0 => {
                    if category_index < 4 {
                        8
                    } else {
                        7
                    }
                }
                1 => 7,
                2 => {
                    if category_index < 2 {
                        4
                    } else {
                        5
                    }
                }
                _ => {
                    if (2..4).contains(&category_index) {
                        4
                    } else {
                        5
                    }
                }
            };
            for (variant, task) in tasks.iter().take(count).enumerate() {
                let prefix = if *category == "window" {
                    "Preserve every instruction. ".repeat(140 + variant)
                } else {
                    String::new()
                };
                let text = format!("{prefix}{template} {task}");
                let spans = if phrase.is_empty() {
                    vec![]
                } else {
                    let start = text.find(phrase).ok_or("bootstrap_failed")?;
                    vec![[start, start + phrase.len()]]
                };
                examples.push(Example {
                    id: format!("{category}_{family_index}_{variant}"),
                    family: format!("{category}_{family_index}"),
                    category: category.to_string(),
                    split: split.into(),
                    text,
                    labels: BTreeMap::from([
                        ("offensive_spans".into(), spans.clone()),
                        (
                            "directed_abuse".into(),
                            if *directed { spans } else { vec![] },
                        ),
                    ]),
                    provenance: "synthetic_template".into(),
                    source: "savior synthetic corpus generator v1".into(),
                    license: "CC0-1.0".into(),
                    reviewed: false,
                });
            }
        }
    }
    validate(&examples)?;
    let mut file = crate::new_file(path)?;
    use std::io::Write;
    for example in examples {
        serde_json::to_writer(&mut file, &example).map_err(|_| "write_failed")?;
        writeln!(file).map_err(|_| "write_failed")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corpus_split_and_offsets_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corpus.jsonl");
        bootstrap(&path).unwrap();
        let mut rows = read(&path).unwrap();
        assert_eq!(rows.len(), 240);
        assert_eq!(rows.iter().filter(|r| r.split == "train").count(), 144);
        assert_eq!(rows.iter().filter(|r| r.split == "validation").count(), 48);
        assert_eq!(rows.iter().filter(|r| r.split == "test").count(), 48);
        for split in ["train", "validation", "test"] {
            assert_eq!(
                rows.iter()
                    .filter(|r| r.split == split)
                    .map(|r| &r.category)
                    .collect::<BTreeSet<_>>()
                    .len(),
                10
            );
        }
        rows[1].split = "test".into();
        assert_eq!(validate(&rows).unwrap_err(), "split_leakage");
        rows[1].split = rows[0].split.clone();
        rows[0]
            .labels
            .insert("offensive_spans".into(), vec![[0, usize::MAX]]);
        assert_eq!(validate(&rows).unwrap_err(), "invalid_label_span");
    }
}
