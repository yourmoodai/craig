use crate::{LabResult, corpus::Example, fail};
use craig::{Detector, FixedMarker, ModelDetector, Policy, Span, apply_replacements};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub examples: usize,
    pub true_positive: usize,
    pub false_positive: usize,
    pub false_negative: usize,
    pub unnecessary_replacements: usize,
    pub abstentions: usize,
    pub unexpected_errors: usize,
    pub deterministic_failures: usize,
    pub exact_outcomes: usize,
}
impl Metrics {
    pub fn precision(&self) -> f64 {
        ratio(self.true_positive, self.true_positive + self.false_positive)
    }
    pub fn recall(&self) -> f64 {
        ratio(self.true_positive, self.true_positive + self.false_negative)
    }
    pub fn f1(&self) -> f64 {
        ratio(
            2 * self.true_positive,
            2 * self.true_positive + self.false_positive + self.false_negative,
        )
    }
    pub fn add(&mut self, other: &Self) {
        self.examples += other.examples;
        self.true_positive += other.true_positive;
        self.false_positive += other.false_positive;
        self.false_negative += other.false_negative;
        self.unnecessary_replacements += other.unnecessary_replacements;
        self.abstentions += other.abstentions;
        self.unexpected_errors += other.unexpected_errors;
        self.deterministic_failures += other.deterministic_failures;
        self.exact_outcomes += other.exact_outcomes;
    }
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"counts": self, "span_precision":self.precision(), "span_recall":self.recall(), "span_f1":self.f1(),
            "exact_outcome_rate":ratio(self.exact_outcomes,self.examples), "exact_outcome_wilson_95":wilson(self.exact_outcomes,self.examples)})
    }
}
fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}
pub fn wilson(success: usize, total: usize) -> [f64; 2] {
    if total == 0 {
        return [0.0, 1.0];
    }
    let n = total as f64;
    let p = success as f64 / n;
    let z = 1.96_f64;
    let denominator = 1.0 + z * z / n;
    let center = (p + z * z / (2.0 * n)) / denominator;
    let width = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / denominator;
    [(center - width).max(0.0), (center + width).min(1.0)]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub id: String,
    pub family: String,
    pub category: String,
    pub status: String,
    pub expected_status: String,
    pub milliseconds: f64,
    pub metrics: Metrics,
    #[serde(skip)]
    pub output: Option<String>,
    #[serde(skip)]
    pub expected: Option<String>,
}
pub fn score(e: &Example, policy: Policy, spans: &[Span], threshold: f32) -> Outcome {
    let expected = apply_replacements(&e.text, &e.spans(policy), 0.5, &FixedMarker::default());
    let actual = apply_replacements(&e.text, spans, threshold, &FixedMarker::default());
    let status = |r: &craig::Result<craig::RewriteResult>| match r {
        Ok(r) => {
            if r.spans.is_empty() {
                "unchanged".into()
            } else {
                "rewritten".into()
            }
        }
        Err(error) => error.to_string(),
    };
    let expected_status = status(&expected);
    let actual_status = status(&actual);
    let mut metrics = Metrics {
        examples: 1,
        ..Default::default()
    };
    if let Ok(result) = &actual {
        let mut cursor = 0;
        let mut reconstructed = String::new();
        let mut valid = result.replacement_count == result.spans.len();
        for span in &result.spans {
            if span.start < cursor
                || span.start >= span.end
                || span.end > e.text.len()
                || !e.text.is_char_boundary(span.start)
                || !e.text.is_char_boundary(span.end)
            {
                valid = false;
                break;
            }
            reconstructed.push_str(&e.text[cursor..span.start]);
            reconstructed.push_str("ya ya ya");
            cursor = span.end;
        }
        if valid {
            reconstructed.push_str(&e.text[cursor..]);
        }
        metrics.deterministic_failures = usize::from(!valid || reconstructed != result.text);
    }
    let expected_spans: BTreeSet<_> = expected
        .as_ref()
        .map(|r| r.spans.iter().map(|s| (s.start, s.end)).collect())
        .unwrap_or_default();
    let actual_spans: BTreeSet<_> = actual
        .as_ref()
        .map(|r| r.spans.iter().map(|s| (s.start, s.end)).collect())
        .unwrap_or_default();
    // Protected abstentions are excluded from span precision and recall.
    if expected.is_ok() {
        metrics.true_positive = expected_spans.intersection(&actual_spans).count();
        metrics.false_positive = actual_spans.difference(&expected_spans).count();
        metrics.false_negative = expected_spans.difference(&actual_spans).count();
        metrics.unnecessary_replacements =
            usize::from(expected_spans.is_empty() && !actual_spans.is_empty());
    }
    metrics.abstentions = usize::from(actual_status == "protected_span_detected");
    metrics.unexpected_errors = usize::from(actual.is_err() && actual_status != expected_status);
    metrics.exact_outcomes = usize::from(match (&actual, &expected) {
        (Ok(a), Ok(b)) => a.text == b.text,
        (Err(a), Err(b)) => a == b,
        _ => false,
    });
    Outcome {
        id: e.id.clone(),
        family: e.family.clone(),
        category: e.category.clone(),
        status: actual_status,
        expected_status,
        milliseconds: 0.0,
        metrics,
        output: actual.ok().map(|r| r.text),
        expected: expected.ok().map(|r| r.text),
    }
}
pub fn run(
    model: &mut ModelDetector,
    rows: &[&Example],
    policy: Policy,
    threshold: f32,
) -> LabResult<Vec<Outcome>> {
    let mut outcomes = Vec::new();
    for row in rows {
        let started = Instant::now();
        let spans = model
            .detect(&row.text)
            .map_err(|_| "evaluation_inference_failed")?;
        let mut outcome = score(row, policy, &spans, threshold);
        outcome.milliseconds = started.elapsed().as_secs_f64() * 1000.0;
        outcomes.push(outcome);
    }
    Ok(outcomes)
}
pub fn summary(outcomes: &[Outcome]) -> serde_json::Value {
    let mut total = Metrics::default();
    let mut categories: BTreeMap<String, Metrics> = BTreeMap::new();
    let mut latencies = Vec::new();
    for o in outcomes {
        total.add(&o.metrics);
        categories
            .entry(o.category.clone())
            .or_default()
            .add(&o.metrics);
        latencies.push(o.milliseconds);
    }
    latencies.sort_by(f64::total_cmp);
    let percentile = |q: f64| {
        latencies
            .get(((latencies.len() as f64 * q).ceil() as usize).saturating_sub(1))
            .copied()
            .unwrap_or(0.0)
    };
    serde_json::json!({"overall":total.summary(),"span_f1_family_bootstrap_95":bootstrap_f1(outcomes),"categories":categories.into_iter().map(|(k,v)|(k,v.summary())).collect::<BTreeMap<_,_>>(),
        "warm_p50_ms":percentile(0.5),"warm_p95_ms":percentile(0.95)})
}
fn bootstrap_f1(outcomes: &[Outcome]) -> [f64; 2] {
    let mut groups: BTreeMap<&str, Metrics> = BTreeMap::new();
    for row in outcomes {
        groups.entry(&row.family).or_default().add(&row.metrics);
    }
    let groups: Vec<_> = groups.values().collect();
    if groups.len() < 2 {
        return [0.0, 1.0];
    }
    let mut samples = Vec::new();
    use sha2::{Digest, Sha256};
    for iteration in 0..1000 {
        let mut total = Metrics::default();
        for draw in 0..groups.len() {
            let digest = Sha256::digest(format!("42:{iteration}:{draw}").as_bytes());
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&digest[..8]);
            total.add(groups[(u64::from_le_bytes(bytes) % groups.len() as u64) as usize]);
        }
        samples.push(total.f1());
    }
    samples.sort_by(f64::total_cmp);
    [samples[25], samples[974]]
}
pub fn calibrate(
    model: &mut ModelDetector,
    rows: &[Example],
    policy: Policy,
) -> LabResult<serde_json::Value> {
    let rows: Vec<_> = rows.iter().filter(|r| r.split == "validation").collect();
    if rows.is_empty() {
        return fail("missing_validation_split");
    }
    let cached: Vec<_> = rows
        .iter()
        .map(|r| {
            model
                .detect(&r.text)
                .map_err(|_| "evaluation_inference_failed".into())
        })
        .collect::<LabResult<_>>()?;
    let mut sweep = Vec::new();
    let mut best = (f64::NEG_INFINITY, f64::NEG_INFINITY, 0.5_f32);
    for step in 1..=19 {
        let threshold = step as f32 * 0.05;
        let mut metrics = Metrics::default();
        for (row, spans) in rows.iter().zip(&cached) {
            metrics.add(&score(row, policy, spans, threshold).metrics);
        }
        if (metrics.f1(), metrics.precision()) > (best.0, best.1) {
            best = (metrics.f1(), metrics.precision(), threshold);
        }
        sweep.push(serde_json::json!({"threshold":threshold,"metrics":metrics.summary()}));
    }
    Ok(
        serde_json::json!({"policy":policy,"selected_threshold":best.2,"selection_split":"validation","sweep":sweep}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_detection_is_not_a_preservation_failure() {
        let row = Example {
            id: "x".into(),
            family: "f".into(),
            category: "directed".into(),
            split: "train".into(),
            text: "You idiot, do not delete the test.".into(),
            labels: BTreeMap::from([("offensive_spans".into(), vec![[4, 9]])]),
            provenance: "synthetic_template".into(),
            source: "fixture".into(),
            license: "CC0-1.0".into(),
            reviewed: false,
        };
        let result = score(&row, Policy::OffensiveSpans, &[], 0.5);
        assert_eq!(result.metrics.false_negative, 1);
        assert_eq!(result.metrics.deterministic_failures, 0);
        assert_eq!(result.metrics.exact_outcomes, 0);
    }
    #[test]
    fn uncertainty_remains_visible() {
        assert_eq!(wilson(0, 0), [0.0, 1.0]);
        assert!(wilson(10, 10)[0] < 0.75);
    }
}
