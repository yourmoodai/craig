//! Offline experiments; held-out labels never enter feature selection or parameter fitting.
#[allow(dead_code)]
#[path = "../corpus.rs"]
mod corpus;
use clap::{Parser, Subcommand};
use middle_finger::{
    Detector, FixedMarker, Span, apply_replacements, artifact_digest,
    decision::{self, Artifact, DecisionDetector},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};
type LabResult<T> = Result<T, String>;
fn fail<T>(s: &str) -> LabResult<T> {
    Err(s.into())
}
fn new_file(path: &Path) -> LabResult<File> {
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
        .map_err(|_| "output_exists_or_unavailable".into())
}
#[derive(Parser)]
#[command(about = "train and evaluate experimental local decision detectors")]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Train {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Evaluate {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        kind: String,
        #[arg(long, default_value = "cpu")]
        backend: String,
        #[arg(long)]
        coreml_dir: Option<PathBuf>,
        #[arg(long, default_value = "float32")]
        precision: String,
        #[arg(long)]
        output: PathBuf,
    },
}
fn gold(row: &corpus::Example) -> Vec<Span> {
    row.spans(middle_finger::Policy::OffensiveSpans)
}
fn mask(text: &str, spans: &[Span]) -> Vec<bool> {
    let mut out = vec![false; text.len()];
    for s in spans {
        out[s.start..s.end].fill(true);
    }
    out
}
fn counts(text: &str, predicted: &[Span], gold: &[Span]) -> [usize; 3] {
    let p = mask(text, predicted);
    let g = mask(text, gold);
    let mut c = [0; 3];
    for (i, _) in text.char_indices() {
        match (p[i], g[i]) {
            (true, true) => c[0] += 1,
            (true, false) => c[1] += 1,
            (false, true) => c[2] += 1,
            _ => {}
        }
    }
    c
}
fn metrics(c: [usize; 3]) -> Value {
    let [tp, fp, miss] = c;
    let precision = if tp + fp == 0 {
        1.
    } else {
        tp as f64 / (tp + fp) as f64
    };
    let recall = if tp + miss == 0 {
        1.
    } else {
        tp as f64 / (tp + miss) as f64
    };
    let f1 = if 2 * tp + fp + miss == 0 {
        1.
    } else {
        2. * tp as f64 / (2 * tp + fp + miss) as f64
    };
    json!({"precision":precision,"recall":recall,"f1":f1,"true_positive_characters":tp,"unnecessary_characters":fp,"missed_characters":miss})
}
fn score(artifact: &Artifact, rows: &[&corpus::Example]) -> LabResult<(f64, usize)> {
    let mut d = DecisionDetector {
        artifact: artifact.clone(),
    };
    let mut c = [0; 3];
    for row in rows {
        let p = d.detect(&row.text).map_err(|e| e.to_string())?;
        let p: Vec<_> = p
            .into_iter()
            .filter(|s| s.score >= artifact.threshold)
            .collect();
        let x = counts(&row.text, &p, &gold(row));
        for i in 0..3 {
            c[i] += x[i];
        }
    }
    Ok((metrics(c)["f1"].as_f64().unwrap(), c[1]))
}
fn select(
    artifact: &Artifact,
    validation: &[&corpus::Example],
) -> LabResult<(Artifact, (f64, usize))> {
    let mut best = artifact.clone();
    let mut quality = (-1., usize::MAX);
    for t in [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9] {
        let mut a = artifact.clone();
        a.threshold = t;
        let s = score(&a, validation)?;
        if s.0 > quality.0 || (s.0 == quality.0 && s.1 < quality.1) {
            best = a;
            quality = s;
        }
    }
    Ok((best, quality))
}
struct Example {
    features: Vec<Vec<usize>>,
    labels: Vec<usize>,
}
fn train(path: &Path, output: &Path) -> LabResult<()> {
    let rows = corpus::read(path)?;
    let train: Vec<_> = rows.iter().filter(|r| r.split == "train").collect();
    let validation: Vec<_> = rows.iter().filter(|r| r.split == "validation").collect();
    if train.is_empty() || validation.is_empty() {
        return fail("empty_training_or_validation_split");
    }
    if output.exists() {
        return fail("output_exists");
    }
    let base = Artifact {
        version: 1,
        kind: "lexical".into(),
        policy: "offensive_spans".into(),
        features: decision::FEATURES.into(),
        threshold: 0.5,
        corpus_sha256: artifact_digest(path).map_err(|e| e.to_string())?,
        epoch: 0,
        labels_reviewed: train.iter().chain(validation.iter()).all(|r| r.reviewed),
        weights: BTreeMap::new(),
        transitions: [[0.; 2]; 2],
    };
    let mut lexical = base.clone();
    lexical.features = decision::LEXICAL_FEATURES.into();
    let mut occurrences: BTreeMap<String, [usize; 2]> = BTreeMap::new();
    let mut vocabulary = BTreeMap::new();
    let mut examples = Vec::new();
    for row in &train {
        let tokens = decision::tokens(&row.text);
        let spans = gold(row);
        let mut example = Example {
            features: vec![],
            labels: vec![],
        };
        for (i, t) in tokens.iter().enumerate() {
            let label = usize::from(spans.iter().any(|s| s.start < t.end && s.end > t.start));
            occurrences.entry(t.word.clone()).or_default()[label] += 1;
            example.labels.push(label);
            let mut f = Vec::new();
            for feature in decision::features(&tokens, i) {
                let next = vocabulary.len();
                f.push(*vocabulary.entry(feature).or_insert(next));
            }
            example.features.push(f);
        }
        examples.push(example);
    }
    for (word, [keep, replace]) in occurrences {
        lexical
            .weights
            .insert(word, (replace + 1) as f64 / (keep + replace + 2) as f64);
    }
    let (lexical, _) = select(&lexical, &validation)?;
    let n = vocabulary.len();
    let mut w = vec![0.; n + 4];
    let mut m = vec![0.; n + 4];
    let mut v = vec![0.; n + 4];
    let mut best = base.clone();
    best.kind = "crf".into();
    let mut best_quality = (-1., usize::MAX);
    for epoch in 1..=50 {
        let mut gradient = vec![0.; n + 4];
        for example in &examples {
            let emissions: Vec<_> = example
                .features
                .iter()
                .map(|fs| fs.iter().map(|&i| w[i]).sum())
                .collect();
            let transition = [[w[n], w[n + 1]], [w[n + 2], w[n + 3]]];
            let result = decision::infer(&emissions, &transition).map_err(|e| e.to_string())?;
            for (i, fs) in example.features.iter().enumerate() {
                for &f in fs {
                    gradient[f] += result.probabilities[i] - example.labels[i] as f64;
                }
            }
            for a in 0..2 {
                for b in 0..2 {
                    gradient[n + a * 2 + b] += result.transition_counts[a][b];
                }
            }
            for labels in example.labels.windows(2) {
                gradient[n + labels[0] * 2 + labels[1]] -= 1.;
            }
        }
        let total_tokens = examples
            .iter()
            .map(|e| e.labels.len())
            .sum::<usize>()
            .max(1) as f64;
        for i in 0..w.len() {
            let g = gradient[i] / total_tokens + 0.0001 * w[i];
            m[i] = 0.9 * m[i] + 0.1 * g;
            v[i] = 0.999 * v[i] + 0.001 * g * g;
            w[i] -= 0.01 * (m[i] / (1. - 0.9f64.powi(epoch)))
                / ((v[i] / (1. - 0.999f64.powi(epoch))).sqrt() + 1e-8);
        }
        let mut a = base.clone();
        a.kind = "crf".into();
        a.epoch = epoch as usize;
        a.transitions = [[w[n], w[n + 1]], [w[n + 2], w[n + 3]]];
        a.weights = vocabulary.iter().map(|(f, &i)| (f.clone(), w[i])).collect();
        let (candidate, quality) = select(&a, &validation)?;
        if quality.0 > best_quality.0 || (quality.0 == best_quality.0 && quality.1 < best_quality.1)
        {
            best = candidate;
            best_quality = quality;
        }
    }
    std::fs::create_dir_all(output).map_err(|_| "write_failed")?;
    for a in [lexical, best] {
        a.validate().map_err(|e| e.to_string())?;
        let dir = output.join(&a.kind);
        std::fs::create_dir(&dir).map_err(|_| "write_failed")?;
        serde_json::to_writer_pretty(new_file(&dir.join("decision.json"))?, &a)
            .map_err(|_| "write_failed")?;
    }
    serde_json::to_writer_pretty(new_file(&output.join("training.json"))?,&json!({"version":1,"seed":0,"order":"corpus_order","optimizer":"full_batch_adam","loss_normalization":"training_token_count","learning_rate":0.01,"l2":0.0001,"epochs":50,"training_examples":train.len(),"validation_examples":validation.len(),"test_used":false,"labels_reviewed":base.labels_reviewed,"corpus_sha256":base.corpus_sha256,"feature_count":n,"policy":"offensive_spans"})).map_err(|_|"write_failed")?;
    Ok(())
}
fn evaluate(
    path: &Path,
    model: &Path,
    kind: &str,
    backend: &str,
    coreml_dir: Option<PathBuf>,
    precision: &str,
    output: &Path,
) -> LabResult<()> {
    let rows = corpus::read(path)?;
    let selected: Vec<_> = rows.iter().filter(|r| r.split == "test").collect();
    if selected.is_empty() {
        return fail("empty_test_split");
    }
    let (mut detector, threshold, model_hash): (Box<dyn Detector>, f32, String) = if kind == "bert"
    {
        let model_hash =
            artifact_digest(&model.join("pytorch_model.bin")).map_err(|e| e.to_string())?;
        let model = middle_finger::ModelDetector::load_policy_with_execution(
            model,
            middle_finger::Policy::OffensiveSpans,
            middle_finger::ExecutionOptions {
                backend: backend.parse().map_err(|_| "invalid_backend")?,
                fp16: match precision {
                    "float32" => false,
                    "float16" => true,
                    _ => return fail("invalid_precision"),
                },
                coreml_dir,
                compute_units: if backend == "core_ml" {
                    middle_finger::ComputeUnits::CpuGpu
                } else {
                    middle_finger::ComputeUnits::All
                },
                window_batch: 1,
            },
        )
        .map_err(|e| e.to_string())?;
        (Box::new(model), 0.5, model_hash)
    } else {
        let d = DecisionDetector::load(model, kind).map_err(|e| e.to_string())?;
        let t = d.threshold();
        (
            Box::new(d),
            t,
            artifact_digest(&model.join("decision.json")).map_err(|e| e.to_string())?,
        )
    };
    let mut total = [0; 3];
    let mut normalized_total = [0; 3];
    let mut normalized_examples = 0;
    let mut details = vec![];
    let mut exact = 0;
    let mut rewrite_matches = 0;
    let mut errors = 0;
    for row in &selected {
        let expected = gold(row);
        let spans = detector.detect(&row.text).map_err(|_| "inference_failed")?;
        let predicted: Vec<_> = spans.into_iter().filter(|s| s.score >= threshold).collect();
        let c = counts(&row.text, &predicted, &expected);
        for i in 0..3 {
            total[i] += c[i];
        }
        let actual = apply_replacements(&row.text, &predicted, threshold, &FixedMarker::default());
        let wanted = apply_replacements(&row.text, &expected, 0.5, &FixedMarker::default());
        if let Ok(wanted) = &wanted {
            let actual_spans = actual.as_ref().map(|r| r.spans.as_slice()).unwrap_or(&[]);
            let counts = counts(&row.text, actual_spans, &wanted.spans);
            for i in 0..3 {
                normalized_total[i] += counts[i];
            }
            normalized_examples += 1;
        }
        let outcome = |r: &middle_finger::Result<middle_finger::RewriteResult>| match r {
            Ok(r) => if r.replacement_count > 0 {
                "rewritten"
            } else {
                "unchanged"
            }
            .to_owned(),
            Err(e) => e.to_string(),
        };
        let fingerprint_value = match &actual {
            Ok(r) => json!([
                "ok",
                r.text,
                r.spans
                    .iter()
                    .map(|s| (
                        middle_finger::byte_to_utf16(&row.text, s.start).unwrap(),
                        middle_finger::byte_to_utf16(&row.text, s.end).unwrap()
                    ))
                    .collect::<Vec<_>>()
            ]),
            Err(e) => json!(["error", e.to_string()]),
        };
        let output_sha256 = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&fingerprint_value).map_err(|_| "serialization_failed")?
            )
        );
        let matches = match (&actual, &wanted) {
            (Ok(a), Ok(b)) => a.text == b.text,
            (Err(a), Err(b)) => a == b,
            _ => false,
        };
        rewrite_matches += usize::from(matches);
        errors += usize::from(
            actual
                .as_ref()
                .is_err_and(|e| *e != middle_finger::Error::ProtectedSpan),
        );
        let p = middle_finger::normalize_spans(&row.text, &predicted, threshold);
        let g = middle_finger::normalize_spans(&row.text, &expected, 0.5);
        let span_comparable = p.is_ok() && g.is_ok();
        let exact_spans = match (p, g) {
            (Ok(p), Ok(g)) => p
                .iter()
                .map(|s| (s.start, s.end))
                .eq(g.iter().map(|s| (s.start, s.end))),
            _ => false,
        };
        exact += usize::from(exact_spans);
        details.push(json!({"id":row.id,"output_sha256":output_sha256,"category":row.category,"reviewed":row.reviewed,"characters":metrics(c),"span_comparable":span_comparable,"exact_spans":exact_spans,"rewrite_match":matches,"actual_outcome":outcome(&actual),"expected_outcome":outcome(&wanted)}));
    }
    serde_json::to_writer_pretty(new_file(output)?,&json!({"version":1,"kind":kind,"backend":backend,"precision":precision,"threshold":threshold,"model_sha256":model_hash,"corpus_sha256":artifact_digest(path).map_err(|e|e.to_string())?,"split":"test","quality_status":"provisional_label_agreement","labels_reviewed":selected.iter().all(|r|r.reviewed),"examples":selected.len(),"character_metric_boundary":"thresholded_detector_spans_before_normalization_and_protection","characters":metrics(total),"normalized_characters":metrics(normalized_total),"normalized_examples":normalized_examples,"normalized_metric_boundary":"expected_success_cases_after_normalization_and_protection","exact_span_eligible_examples":details.iter().filter(|r|r["span_comparable"]==true).count(),"exact_span_matches":exact,"exact_rewrite_matches":rewrite_matches,"errors":errors,"results":details})).map_err(|_|"write_failed")?;
    Ok(())
}
fn main() {
    let result = match Args::parse().command {
        Command::Train { corpus, output } => train(&corpus, &output),
        Command::Evaluate {
            corpus,
            model,
            kind,
            backend,
            coreml_dir,
            precision,
            output,
        } => evaluate(
            &corpus, &model, &kind, &backend, coreml_dir, &precision, &output,
        ),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod decision_tests {
    use super::*;
    #[test]
    fn heldout_labels_cannot_change_trained_parameters() {
        let dir = tempfile::tempdir().unwrap();
        let rows:Vec<_>=[("train","train_bad","You idiot.",vec![[4,9]]),("train","train_clean","Keep tests.",vec![]),("validation","validation_case","Fix tests, idiot.",vec![[11,16]]),("test","test_case","A moron.",vec![[2,7]])].into_iter().map(|(split,id,text,spans)|json!({"id":id,"family":id,"category":"synthetic","split":split,"text":text,"labels":{"offensive_spans":spans,"directed_abuse":[]},"provenance":"synthetic_template","source":"decision unit test","license":"CC0-1.0","reviewed":false})).collect();
        let corpus = dir.path().join("corpus.jsonl");
        let write = |rows: &[Value]| {
            std::fs::write(
                &corpus,
                rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
            )
            .unwrap()
        };
        write(&rows);
        train(&corpus, &dir.path().join("first")).unwrap();
        let mut changed = rows;
        changed[3]["labels"]["offensive_spans"] = json!([]);
        write(&changed);
        train(&corpus, &dir.path().join("second")).unwrap();
        for kind in ["lexical", "crf"] {
            let a = DecisionDetector::load(dir.path().join("first").join(kind), kind)
                .unwrap()
                .artifact;
            let b = DecisionDetector::load(dir.path().join("second").join(kind), kind)
                .unwrap()
                .artifact;
            assert_eq!(a.weights, b.weights);
            assert_eq!(a.transitions, b.transitions);
            assert_eq!(a.threshold, b.threshold);
            assert_eq!(a.epoch, b.epoch);
        }
        assert!(train(&corpus, &dir.path().join("first")).is_err());
    }
}
