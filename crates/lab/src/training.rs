use crate::{
    LabResult,
    corpus::Example,
    evaluation::{self, Metrics},
    fail,
};
use candle_core::{DType, Device, Module, Tensor};
use candle_nn::{
    Linear, Optimizer, ParamsAdamW, VarBuilder, VarMap, linear,
    ops::{log_softmax, softmax_last_dim},
};
use savior::{
    FixedMarker, MODEL_ID, MODEL_REVISION, ModelDetector, ModelManifest, Policy, Span,
    artifact_digest,
};
use std::{collections::BTreeMap, path::Path};

pub const BASE_FILES: [&str; 5] = [
    "config.json",
    "model_args.json",
    "tokenizer_config.json",
    "vocab.txt",
    "pytorch_model.bin",
];
fn tensor<T>(result: candle_core::Result<T>) -> LabResult<T> {
    result.map_err(|_| "training_tensor_failed".into())
}
struct Window {
    hidden: Tensor,
    labels: Tensor,
    weights: Tensor,
    offsets: Vec<(usize, usize)>,
}
struct Cached<'a> {
    example: &'a Example,
    windows: Vec<Window>,
}
fn cache<'a>(
    model: &ModelDetector,
    rows: &'a [Example],
    policy: Policy,
) -> LabResult<Vec<Cached<'a>>> {
    let mut cached = Vec::new();
    for example in rows.iter().filter(|r| r.split != "test") {
        let windows = model
            .training_windows(&example.text)
            .map_err(|_| "training_inference_failed")?;
        let mut counts = BTreeMap::new();
        for window in &windows {
            for offset in &window.offsets {
                *counts.entry(*offset).or_insert(0usize) += 1;
            }
        }
        let gold = example.spans(policy);
        let mut result = Vec::new();
        for window in windows {
            let labels: Vec<u32> = window
                .offsets
                .iter()
                .map(|&(a, b)| u32::from(gold.iter().any(|s| a < s.end && b > s.start)))
                .collect();
            let weights: Vec<f32> = window
                .offsets
                .iter()
                .map(|&(a, b)| {
                    if a < b {
                        1.0 / counts[&(a, b)] as f32
                    } else {
                        0.0
                    }
                })
                .collect();
            result.push(Window {
                hidden: window.hidden,
                labels: tensor(Tensor::new(labels.as_slice(), &Device::Cpu))?,
                weights: tensor(Tensor::new(weights.as_slice(), &Device::Cpu))?,
                offsets: window.offsets,
            });
        }
        cached.push(Cached {
            example,
            windows: result,
        });
    }
    Ok(cached)
}
fn predictions(head: &Linear, row: &Cached<'_>) -> LabResult<Vec<Span>> {
    let mut spans = Vec::new();
    for window in &row.windows {
        let probabilities = tensor(softmax_last_dim(&tensor(head.forward(&window.hidden))?))?;
        let values = tensor(probabilities.to_vec2::<f32>())?;
        for (&(start, end), value) in window.offsets.iter().zip(values) {
            if !value.iter().all(|v| v.is_finite()) {
                return fail("nonfinite_training_output");
            }
            if start < end {
                spans.push(Span {
                    start,
                    end,
                    score: value[1],
                });
            }
        }
    }
    Ok(spans)
}
fn validation(head: &Linear, cached: &[Cached<'_>], policy: Policy) -> LabResult<(f64, f64, f32)> {
    let rows: Vec<_> = cached
        .iter()
        .filter(|r| r.example.split == "validation")
        .collect();
    if rows.is_empty() {
        return fail("missing_validation_split");
    }
    let predicted: Vec<_> = rows
        .iter()
        .map(|r| predictions(head, r))
        .collect::<LabResult<_>>()?;
    let mut best = (-1.0, -1.0, 0.5);
    for step in 1..=19 {
        let threshold = step as f32 * 0.05;
        let mut metrics = Metrics::default();
        for (row, spans) in rows.iter().zip(&predicted) {
            metrics.add(
                &evaluation::score(
                    row.example,
                    policy,
                    spans,
                    threshold,
                    &FixedMarker::default(),
                )
                .metrics,
            );
        }
        if (metrics.f1(), metrics.precision()) > (best.0, best.1) {
            best = (metrics.f1(), metrics.precision(), threshold);
        }
    }
    Ok(best)
}
pub fn train(
    base: &Path,
    rows: &[Example],
    corpus_hash: &str,
    policy: Policy,
    output: &Path,
    epochs: usize,
) -> LabResult<serde_json::Value> {
    if !(1..=10).contains(&epochs) {
        return fail("invalid_epochs");
    }
    if base.join("manifest.json").exists() {
        return fail("training_requires_pinned_base");
    }
    let model = ModelDetector::load(base).map_err(|_| "base_model_unavailable")?;
    let encoder_hash =
        artifact_digest(&base.join("pytorch_model.bin")).map_err(|_| "base_model_unavailable")?;
    let cached = cache(&model, rows, policy)?;
    let mut training: Vec<_> = cached
        .iter()
        .filter(|r| r.example.split == "train")
        .collect();
    if training.is_empty() {
        return fail("missing_training_split");
    }
    std::fs::create_dir(output).map_err(|_| "output_exists_or_unavailable")?;
    let mut variables = VarMap::new();
    let head = tensor(linear(
        768,
        2,
        VarBuilder::from_varmap(&variables, DType::F32, &Device::Cpu).pp("classifier"),
    ))?;
    let source = tensor(VarBuilder::from_pth(
        base.join("pytorch_model.bin"),
        DType::F32,
        &Device::Cpu,
    ))?;
    tensor(variables.set_one(
        "classifier.weight",
        tensor(source.get((2, 768), "classifier.weight"))?,
    ))?;
    tensor(variables.set_one("classifier.bias", tensor(source.get(2, "classifier.bias"))?))?;
    let initial = tensor(head.weight().to_vec2::<f32>())?;
    let mut optimizer = tensor(candle_nn::AdamW::new(
        variables.all_vars(),
        ParamsAdamW {
            lr: 0.001,
            ..Default::default()
        },
    ))?;
    let mut best = (-1.0, -1.0, 0.5);
    let mut history = Vec::new();
    use sha2::{Digest, Sha256};
    for epoch in 0..epochs {
        training.sort_by_key(|r| {
            Sha256::digest(format!("42:{epoch}:{}", r.example.id).as_bytes()).to_vec()
        });
        let mut losses = 0.0;
        for row in &training {
            let mut total_loss: Option<Tensor> = None;
            let mut total_weight = 0.0_f64;
            for window in &row.windows {
                let log_probs = tensor(log_softmax(&tensor(head.forward(&window.hidden))?, 1))?;
                let selected = tensor(log_probs.gather(&tensor(window.labels.unsqueeze(1))?, 1))?;
                let loss =
                    tensor(tensor(tensor(selected.squeeze(1))?.neg())?.mul(&window.weights))?;
                let loss = tensor(loss.sum_all())?;
                total_weight +=
                    tensor(tensor(window.weights.sum_all())?.to_scalar::<f32>())? as f64;
                total_loss = Some(match total_loss {
                    Some(previous) => tensor(previous.add(&loss))?,
                    None => loss,
                });
            }
            if total_weight <= 0.0 {
                return fail("empty_training_tokens");
            }
            let loss = tensor(
                total_loss
                    .ok_or("empty_training_tokens")?
                    .affine(1.0 / total_weight, 0.0),
            )?;
            let value = tensor(loss.to_scalar::<f32>())?;
            if !value.is_finite() {
                return fail("nonfinite_training_loss");
            }
            losses += value as f64;
            tensor(optimizer.backward_step(&loss))?;
        }
        let current = validation(&head, &cached, policy)?;
        history.push(serde_json::json!({"epoch":epoch+1,"loss":losses/training.len() as f64,"validation_f1":current.0,"validation_precision":current.1,"threshold":current.2}));
        if (current.0, current.1) > (best.0, best.1) {
            best = current;
            tensor(variables.save(output.join("head.safetensors")))?;
        }
    }
    tensor(variables.load(output.join("head.safetensors")))?;
    if tensor(head.weight().to_vec2::<f32>())? == initial {
        return fail("classifier_unchanged");
    }
    if artifact_digest(&base.join("pytorch_model.bin")).map_err(|_| "base_model_unavailable")?
        != encoder_hash
    {
        return fail("encoder_changed");
    }
    for file in BASE_FILES {
        std::fs::copy(base.join(file), output.join(file)).map_err(|_| "artifact_copy_failed")?;
    }
    let manifest = ModelManifest {
        version: 1,
        base_model: MODEL_ID.into(),
        base_revision: MODEL_REVISION.into(),
        policy,
        threshold: best.2,
        labels: vec!["NOT_TOXIC".into(), "TOXIC".into()],
        head_sha256: artifact_digest(&output.join("head.safetensors"))
            .map_err(|_| "artifact_copy_failed")?,
        provenance: serde_json::json!({"corpus_sha256":corpus_hash,"encoder_sha256":encoder_hash,"seed":42,"optimizer":"adamw","learning_rate":0.001,
            "batch_size":1,"epochs":epochs,"encoder_frozen":true,"labels_reviewed":rows.iter().filter(|r|r.split!="test").all(|r|r.reviewed),"history":history,
            "dataset_licenses":rows.iter().map(|r|r.license.clone()).collect::<std::collections::BTreeSet<_>>(),"dataset_sources":rows.iter().map(|r|r.source.clone()).collect::<std::collections::BTreeSet<_>>()}),
    };
    crate::write_json(&output.join("manifest.json"), &manifest)?;
    let mut reloaded =
        ModelDetector::load_policy(output, policy).map_err(|_| "export_reload_failed")?;
    use savior::Detector;
    let mut maximum = 0.0_f32;
    for row in cached.iter().filter(|r| r.example.split == "validation") {
        let before = predictions(&head, row)?;
        let after = reloaded
            .detect(&row.example.text)
            .map_err(|_| "export_reload_failed")?;
        if before.len() != after.len() {
            return fail("export_parity_failed");
        }
        for (a, b) in before.iter().zip(after) {
            if (a.start, a.end) != (b.start, b.end) {
                return fail("export_parity_failed");
            }
            maximum = maximum.max((a.score - b.score).abs());
        }
    }
    if maximum > 0.0001 {
        return fail("export_parity_failed");
    }
    let report = serde_json::json!({"policy":policy,"encoder_unchanged":true,"classifier_changed":true,"export_probability_max_error":maximum,"validation_f1":best.0,"selected_threshold":best.2,"history":history,"status":"candidate_not_promoted"});
    crate::write_json(&output.join("training.json"), &report)?;
    Ok(report)
}
