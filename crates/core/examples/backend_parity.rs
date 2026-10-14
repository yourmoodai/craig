use craig::{Detector, ExecutionOptions, FixedMarker, ModelDetector, Policy, apply_replacements};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let backend = std::env::var("CRAIG_BENCH_BACKEND").unwrap_or_else(|_| "cpu".into());
    if !["cpu", "metal", "core_ml"].contains(&backend.as_str()) {
        return Err("invalid_backend".into());
    }
    let dir =
        std::env::var("CRAIG_MODEL_DIR").unwrap_or_else(|_| ".cache/models/mudes-en-base".into());
    let mut reference = ModelDetector::load(&dir)?;
    let mut candidate = ModelDetector::load_policy_with_execution(
        &dir,
        Policy::OffensiveSpans,
        ExecutionOptions {
            backend: backend.parse()?,
            coreml_dir: std::env::var("CRAIG_COREML_DIR").ok().map(Into::into),
            compute_units: std::env::var("CRAIG_COMPUTE_UNITS")
                .unwrap_or_else(|_| "all".into())
                .parse()?,
            fp16: std::env::var("CRAIG_BENCH_PRECISION").as_deref() == Ok("float16"),
            window_batch: std::env::var("CRAIG_BENCH_BATCH")
                .unwrap_or_else(|_| "4".into())
                .parse()?,
        },
    )?;
    let rows: serde_json::Value =
        serde_json::from_slice(&std::fs::read("tests/fixtures/model.json")?)?;
    let mut texts: Vec<String> = rows
        .as_array()
        .ok_or("invalid_fixtures")?
        .iter()
        .map(|r| r["text"].as_str().unwrap().to_owned())
        .collect();
    texts.push(format!(
        "{} You stupid idiot! {}",
        "Please preserve these tests. ".repeat(180),
        "🙂 keep `idiot_name` unchanged. ".repeat(12)
    ));
    texts.push(format!(
        "{} You idiot, keep the test.",
        "Please preserve these tests. ".repeat(680)
    ));
    let mut maximum = 0.0f32;
    let mut outcome_mismatches = 0;
    for text in &texts {
        let expected = reference.detect(text)?;
        let actual = candidate.detect(text)?;
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.iter().zip(&expected) {
            assert_eq!((a.start, a.end), (b.start, b.end));
            maximum = maximum.max((a.score - b.score).abs());
            assert!(a.score.is_finite(), "nonfinite probability");
        }
        let output = |spans: &[_]| {
            apply_replacements(text, spans, 0.5, &FixedMarker::default()).map(|r| r.text)
        };
        if output(&actual) != output(&expected) {
            outcome_mismatches += 1;
        }
    }
    println!(
        "backend parity: {} fixtures, maximum probability error {maximum}, outcome mismatches {outcome_mismatches}",
        texts.len()
    );
    if maximum >= 0.0001 || outcome_mismatches != 0 {
        return Err("backend_parity_failed".into());
    }
    Ok(())
}
