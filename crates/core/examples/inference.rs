use middle_finger::{
    Detector, ExecutionOptions, FixedMarker, ModelDetector, Policy, apply_replacements, rewrite,
};
use serde::Deserialize;
use serde_json::json;
use std::{io::Read, time::Instant};

enum Instance {
    Bert(Box<ModelDetector>),
    Decision(middle_finger::decision::DecisionDetector),
}
impl Detector for Instance {
    fn detect(&mut self, text: &str) -> middle_finger::Result<Vec<middle_finger::Span>> {
        match self {
            Self::Bert(d) => d.detect(text),
            Self::Decision(d) => d.detect(text),
        }
    }
}
impl Instance {
    fn profile(
        &mut self,
        text: &str,
    ) -> middle_finger::Result<(Vec<middle_finger::Span>, serde_json::Value)> {
        match self {
            Self::Bert(d) => d.detect_profiled(text).map(|(s, t)| (s, json!(t))),
            Self::Decision(d) => {
                let start = Instant::now();
                let s = d.detect(text)?;
                Ok((
                    s,
                    json!({"detection_ms":start.elapsed().as_secs_f64()*1000.}),
                ))
            }
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    id: String,
    text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    model: String,
    #[serde(default)]
    order: Option<String>,
    #[serde(default = "default_detector")]
    detector: String,
    backend: String,
    #[serde(default)]
    fp16: bool,
    coreml_dir: Option<String>,
    #[serde(default = "default_compute")]
    compute_units: String,
    batch: usize,
    samples: usize,
    warmup: usize,
    fixtures: Vec<Fixture>,
}
fn default_detector() -> String {
    "bert".into()
}
fn default_compute() -> String {
    "all".into()
}
fn peak_rss_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // Safety: successful getrusage initializes the supplied storage.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            let value = unsafe { usage.assume_init() }.ru_maxrss as u64;
            return Some(if cfg!(target_os = "macos") {
                value
            } else {
                value * 1024
            });
        }
    }
    None
}
#[allow(dead_code)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(None)
}

pub fn run(allocations: Option<fn() -> (u64, u64)>) -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = String::new();
    std::io::stdin()
        .take(8_388_609)
        .read_to_string(&mut bytes)?; // 8 MiB plus overflow sentinel
    if bytes.len() > 8_388_608 {
        return Err("invalid_request".into());
    }
    let request: Request = serde_json::from_str(&bytes)?;
    if !["cpu", "metal", "core_ml"].contains(&request.backend.as_str())
        || request.samples == 0
        || request.samples > 100_000
        || request.warmup > 1000
        || request.fixtures.is_empty()
    {
        return Err("invalid_options".into());
    }
    let start = Instant::now();
    let (mut detector, threshold): (Instance, f32) = if request.detector == "bert" {
        (
            Instance::Bert(Box::new(ModelDetector::load_policy_with_execution(
                &request.model,
                Policy::OffensiveSpans,
                ExecutionOptions {
                    backend: request.backend.parse()?,
                    coreml_dir: request.coreml_dir.map(Into::into),
                    compute_units: request.compute_units.parse()?,
                    fp16: request.fp16,
                    window_batch: request.batch,
                },
            )?)),
            0.5,
        )
    } else {
        if request.backend != "cpu" || request.fp16 || request.batch != 1 {
            return Err("invalid_options".into());
        }
        let d = middle_finger::decision::DecisionDetector::load(&request.model, &request.detector)?;
        let threshold = d.threshold();
        (Instance::Decision(d), threshold)
    };
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    let marker = FixedMarker::default();
    let mut results = Vec::new();
    for fixture in &request.fixtures {
        results.push(json!({"id":fixture.id,"bytes":fixture.text.len(),"times_ms":[],"outcomes":{},"errors":{}}));
    }
    let mut measure = |i: usize, record: bool| {
        let fixture = &request.fixtures[i];
        let row = &mut results[i];
        let start = Instant::now();
        let result = rewrite(&mut detector, &fixture.text, threshold, &marker);
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        if !record {
            return;
        }
        let key = match result {
            Ok(r) => {
                if r.replacement_count == 0 {
                    "unchanged".to_owned()
                } else {
                    "rewritten".to_owned()
                }
            }
            Err(e) => e.to_string(),
        };
        if !["unchanged", "rewritten", "protected_span_detected"].contains(&key.as_str()) {
            let count = row["errors"][&key].as_u64().unwrap_or(0);
            row["errors"][&key] = json!(count + 1);
        }
        let count = row["outcomes"][&key].as_u64().unwrap_or(0);
        row["outcomes"][&key] = json!(count + 1);
        row["times_ms"].as_array_mut().unwrap().push(json!(elapsed));
    };
    match request.order.as_deref().unwrap_or("fixture") {
        "fixture" => {
            for i in 0..request.fixtures.len() {
                for n in 0..request.warmup + request.samples {
                    measure(i, n >= request.warmup);
                }
            }
        }
        "interleaved" => {
            for n in 0..request.warmup + request.samples {
                for i in 0..request.fixtures.len() {
                    measure(i, n >= request.warmup);
                }
            }
        }
        _ => return Err("invalid_order".into()),
    }
    for (fixture, row) in request.fixtures.iter().zip(results.iter_mut()) {
        let before = allocations.map(|f| f());
        let profile = detector.profile(&fixture.text);
        let after = allocations.map(|f| f());
        let (stages, replacement_ms, error) = match profile {
            Ok((spans, stages)) => {
                let start = Instant::now();
                let _ = apply_replacements(&fixture.text, &spans, threshold, &marker);
                (stages, json!(start.elapsed().as_secs_f64() * 1000.), None)
            }
            Err(e) => (json!(null), json!(null), Some(e.to_string())),
        };
        row["stages"] = stages;
        row["replacement_only_ms"] = replacement_ms;
        row["profile_error"] = json!(error);
        row["profile_allocations"] = json!(before.zip(after).map(|(a, b)| b.0 - a.0));
        row["profile_allocated_bytes"] = json!(before.zip(after).map(|(a, b)| b.1 - a.1));
    }
    println!(
        "{}",
        json!({"allocation_instrumented":allocations.is_some(),"detector":request.detector,"threshold":threshold,"backend":request.backend,"fp16":request.fp16,"compute_units":request.compute_units,"accelerate":cfg!(feature="accelerate"),"window_batch":request.batch,"load_ms":load_ms,"peak_rss_bytes":peak_rss_bytes(),"results":results})
    );
    Ok(())
}
