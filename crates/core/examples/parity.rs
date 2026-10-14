use craig::{Detector, ExecutionOptions, ModelDetector, Policy, Span};
use serde::Deserialize;
#[derive(Deserialize)]
struct Fixture {
    text: String,
    spans: Vec<Span>,
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let backend = std::env::var("CRAIG_BENCH_BACKEND").unwrap_or_else(|_| "cpu".into());
    if !["cpu", "metal", "core_ml"].contains(&backend.as_str()) {
        return Err("invalid_backend".into());
    }
    let dir =
        std::env::var("CRAIG_MODEL_DIR").unwrap_or_else(|_| ".cache/models/mudes-en-base".into());
    let fixtures: Vec<Fixture> = serde_json::from_slice(&std::fs::read(".cache/reference.json")?)?;
    let mut model = ModelDetector::load_policy_with_execution(
        dir,
        Policy::OffensiveSpans,
        ExecutionOptions {
            backend: backend.parse()?,
            coreml_dir: std::env::var("CRAIG_COREML_DIR").ok().map(Into::into),
            compute_units: std::env::var("CRAIG_COMPUTE_UNITS")
                .unwrap_or_else(|_| "all".into())
                .parse()?,
            fp16: std::env::var("CRAIG_BENCH_PRECISION").as_deref() == Ok("float16"),
            window_batch: std::env::var("CRAIG_BENCH_BATCH")
                .unwrap_or_else(|_| "1".into())
                .parse()?,
        },
    )?;
    let mut maximum = 0.0f32;
    for fixture in &fixtures {
        let actual = model.detect(&fixture.text)?;
        assert_eq!(actual.len(), fixture.spans.len(), "token count mismatch");
        for (a, b) in actual.iter().zip(&fixture.spans) {
            assert_eq!((a.start, a.end), (b.start, b.end), "token offset mismatch");
            let difference = (a.score - b.score).abs();
            maximum = maximum.max(difference);
            assert!(difference < 0.0001, "probability mismatch: {difference}");
        }
    }
    println!(
        "reference parity passed: {} fixtures, maximum probability error {maximum}",
        fixtures.len()
    );
    Ok(())
}
