use crate::bert::Bert;
use crate::{Detector, Error, ModelManifest, Policy, Result, Span, validate_text};
use candle_core::{DType, Device, Module, Tensor};
use candle_nn::{Linear, VarBuilder, linear, ops::softmax_last_dim};
use candle_transformers::models::bert::Config;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tokenizers::{
    Tokenizer, models::wordpiece::WordPiece, normalizers::bert::BertNormalizer,
    pre_tokenizers::bert::BertPreTokenizer,
};

fn verify_artifacts(dir: &Path) -> Result<()> {
    for artifact in &crate::model_catalog::delivery_detector().artifacts {
        let name = &artifact.file;
        let expected = &artifact.sha256;
        let mut file = std::fs::File::open(dir.join(name)).map_err(|_| Error::ModelUnavailable)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65_536]; // 64 KiB
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|_| Error::ModelUnavailable)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        if format!("{:x}", hash.finalize()) != *expected {
            return Err(Error::ModelIncompatible);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    #[default]
    Cpu,
    Metal,
    CoreMl,
}
impl std::str::FromStr for Backend {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "cpu" => Ok(Self::Cpu),
            "metal" => Ok(Self::Metal),
            "core_ml" => Ok(Self::CoreMl),
            _ => Err(Error::InvalidOptions),
        }
    }
}
#[derive(Clone, Debug)]
pub struct ExecutionOptions {
    pub backend: Backend,
    pub coreml_dir: Option<std::path::PathBuf>,
    pub compute_units: crate::ComputeUnits,
    pub fp16: bool,
    pub window_batch: usize,
}
impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            backend: Backend::Cpu,
            coreml_dir: None,
            compute_units: crate::ComputeUnits::All,
            fp16: false,
            window_batch: 1,
        }
    }
}
#[derive(Default, Debug, serde::Serialize)]
pub struct InferenceTimings {
    pub tokens: usize,
    pub windows: usize,
    pub tokenize_ms: f64,
    pub encoder_ms: f64,
    pub classifier_ms: f64,
    pub offsets_ms: f64,
}
pub struct ModelDetector {
    device: Device,
    window_batch: usize,
    tokenizer: Tokenizer,
    encoder: Option<Bert>,
    classifier: Option<Linear>,
    coreml: Option<crate::coreml::CoreMl>,
    cls: u32,
    sep: u32,
    threshold: f32,
}
impl ModelDetector {
    /// Loads local artifacts only. No runtime network client is linked into this crate.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        Self::load_policy(dir, Policy::OffensiveSpans)
    }
    pub fn load_policy(dir: impl AsRef<Path>, policy: Policy) -> Result<Self> {
        Self::load_policy_with_execution(dir, policy, ExecutionOptions::default())
    }
    pub fn load_policy_with_execution(
        dir: impl AsRef<Path>,
        policy: Policy,
        execution: ExecutionOptions,
    ) -> Result<Self> {
        if ![1, 2, 4, 8].contains(&execution.window_batch)
            || (execution.fp16 && execution.backend == Backend::Cpu)
            || (execution.backend != Backend::CoreMl
                && (execution.coreml_dir.is_some()
                    || execution.compute_units != crate::ComputeUnits::All))
            || (execution.backend == Backend::CoreMl && execution.coreml_dir.is_none())
        {
            return Err(Error::InvalidOptions);
        }
        let device = if execution.backend == Backend::Metal {
            Device::new_metal(0).map_err(|_| Error::ModelUnavailable)?
        } else {
            Device::Cpu
        };
        let dir = dir.as_ref();
        verify_artifacts(dir)?;
        let manifest = if dir
            .join("manifest.json")
            .try_exists()
            .map_err(|_| Error::ModelUnavailable)?
        {
            Some(ModelManifest::read(dir)?)
        } else {
            None
        };
        if manifest.as_ref().map(|m| m.policy).unwrap_or_default() != policy {
            return Err(Error::PolicyUnavailable);
        }
        let read =
            |name| std::fs::read_to_string(dir.join(name)).map_err(|_| Error::ModelUnavailable);
        let config: Config =
            serde_json::from_str(&read("config.json")?).map_err(|_| Error::ModelIncompatible)?;
        let args: serde_json::Value = serde_json::from_str(&read("model_args.json")?)
            .map_err(|_| Error::ModelIncompatible)?;
        let tokenizer_config: serde_json::Value =
            serde_json::from_str(&read("tokenizer_config.json")?)
                .map_err(|_| Error::ModelIncompatible)?;
        if args["labels_list"] != serde_json::json!(["NOT_TOXIC", "TOXIC"])
            || config.hidden_size != 768
            || config.max_position_embeddings != 512
            || tokenizer_config["do_lower_case"] != false
        {
            return Err(Error::ModelIncompatible);
        }
        let vocab = dir.join("vocab.txt");
        let wordpiece = WordPiece::from_file(vocab.to_str().ok_or(Error::ModelUnavailable)?)
            .unk_token("[UNK]".into())
            .build()
            .map_err(|_| Error::ModelIncompatible)?;
        let mut tokenizer = Tokenizer::new(wordpiece);
        tokenizer.with_normalizer(Some(BertNormalizer::new(true, true, None, false)));
        tokenizer.with_pre_tokenizer(Some(BertPreTokenizer));
        let cls = tokenizer
            .token_to_id("[CLS]")
            .ok_or(Error::ModelIncompatible)?;
        let sep = tokenizer
            .token_to_id("[SEP]")
            .ok_or(Error::ModelIncompatible)?;
        let (encoder, classifier, coreml) = if execution.backend == Backend::CoreMl {
            let converted = execution
                .coreml_dir
                .as_deref()
                .ok_or(Error::InvalidOptions)?;
            (
                None,
                None,
                Some(crate::coreml::CoreMl::load(
                    converted,
                    dir,
                    execution.fp16,
                    execution.compute_units,
                )?),
            )
        } else {
            let dtype = if execution.fp16 {
                DType::F16
            } else {
                DType::F32
            };
            let weights = VarBuilder::from_pth(dir.join("pytorch_model.bin"), dtype, &device)
                .map_err(|_| Error::ModelUnavailable)?;
            let encoder =
                Bert::load(weights.pp("bert"), &config).map_err(|_| Error::ModelIncompatible)?;
            let head = if manifest.is_some() {
                let bytes = std::fs::read(dir.join("head.safetensors"))
                    .map_err(|_| Error::ModelUnavailable)?;
                VarBuilder::from_buffered_safetensors(bytes, dtype, &device)
                    .map_err(|_| Error::ModelIncompatible)?
            } else {
                weights.clone()
            };
            let classifier = linear(config.hidden_size, 2, head.pp("classifier"))
                .map_err(|_| Error::ModelIncompatible)?;
            if manifest.is_some() {
                for tensor in std::iter::once(classifier.weight()).chain(classifier.bias()) {
                    if tensor
                        .flatten_all()
                        .and_then(|t| t.to_dtype(DType::F32))
                        .and_then(|t| t.to_vec1::<f32>())
                        .map_err(|_| Error::ModelIncompatible)?
                        .iter()
                        .any(|v| !v.is_finite())
                    {
                        return Err(Error::ModelIncompatible);
                    }
                }
            }
            (Some(encoder), Some(classifier), None)
        };
        Ok(Self {
            device,
            window_batch: execution.window_batch,
            tokenizer,
            encoder,
            classifier,
            coreml,
            cls,
            sep,
            threshold: manifest.map(|m| m.threshold).unwrap_or(0.5),
        })
    }
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    fn encode_windows(&self, windows: &[&[u32]]) -> Result<Tensor> {
        let width = windows.iter().map(|tokens| tokens.len()).max().unwrap_or(0) + 2;
        let mut ids = vec![0u32; windows.len() * width];
        let mut mask = vec![0u32; ids.len()];
        for (i, tokens) in windows.iter().enumerate() {
            let start = i * width;
            ids[start] = self.cls;
            ids[start + 1..start + 1 + tokens.len()].copy_from_slice(tokens);
            ids[start + tokens.len() + 1] = self.sep;
            mask[start..start + tokens.len() + 2].fill(1);
        }
        let input = Tensor::from_vec(ids, (windows.len(), width), &self.device)
            .map_err(|_| Error::InferenceFailed)?;
        let mask = Tensor::from_vec(mask, (windows.len(), width), &self.device)
            .map_err(|_| Error::InferenceFailed)?;
        let types = input.zeros_like().map_err(|_| Error::InferenceFailed)?;
        self.encoder
            .as_ref()
            .ok_or(Error::InvalidOptions)?
            .forward(&input, &types, &mask)
            .map_err(|_| Error::InferenceFailed)
    }

    #[cfg(feature = "training")]
    fn encode_window(&self, tokens: &[u32]) -> Result<Tensor> {
        self.encode_windows(&[tokens])?
            .squeeze(0)
            .and_then(|t| t.narrow(0, 1, tokens.len()))
            .map_err(|_| Error::InferenceFailed)
    }

    pub fn detect_profiled(&mut self, text: &str) -> Result<(Vec<Span>, InferenceTimings)> {
        let mut timings = InferenceTimings::default();
        let spans = self.detect_inner(text, Some(&mut timings))?;
        Ok((spans, timings))
    }
    #[cfg(feature = "training")]
    pub fn training_windows(&self, text: &str) -> Result<Vec<TrainingWindow>> {
        validate_text(text)?;
        let encoded = self
            .tokenizer
            .encode(text, false)
            .map_err(|_| Error::InferenceFailed)?;
        let mut result = Vec::new();
        for range in window_ranges(encoded.len()) {
            let hidden = self
                .encode_window(&encoded.get_ids()[range.clone()])?
                .detach();
            result.push(TrainingWindow {
                hidden,
                offsets: encoded.get_offsets()[range].to_vec(),
            });
        }
        Ok(result)
    }
}

fn window_ranges(length: usize) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < length {
        let end = (start + 510).min(length);
        ranges.push(start..end);
        if end == length {
            break;
        }
        start = end - 128;
    }
    ranges
}

#[cfg(feature = "training")]
pub struct TrainingWindow {
    pub hidden: Tensor,
    pub offsets: Vec<(usize, usize)>,
}
impl Detector for ModelDetector {
    fn detect(&mut self, text: &str) -> Result<Vec<Span>> {
        self.detect_inner(text, None)
    }
}
impl ModelDetector {
    fn detect_inner(
        &mut self,
        text: &str,
        mut timings: Option<&mut InferenceTimings>,
    ) -> Result<Vec<Span>> {
        validate_text(text)?;
        let started = Instant::now();
        let encoded = self
            .tokenizer
            .encode(text, false)
            .map_err(|_| Error::InferenceFailed)?;
        let ids = encoded.get_ids();
        let offsets = encoded.get_offsets();
        let ranges = window_ranges(ids.len());
        if let Some(t) = timings.as_deref_mut() {
            t.tokenize_ms = started.elapsed().as_secs_f64() * 1000.0;
            t.tokens = ids.len();
            t.windows = ranges.len();
        }
        let mut spans = Vec::new();
        for batch in ranges.chunks(self.window_batch) {
            if started.elapsed() > Duration::from_secs(30) {
                return Err(Error::Timeout);
            }
            let stage = Instant::now();
            let windows: Vec<_> = batch.iter().map(|range| &ids[range.clone()]).collect();
            let probabilities = if let Some(coreml) = &self.coreml {
                let probabilities = coreml.predict(&windows, self.cls, self.sep)?;
                if let Some(t) = timings.as_deref_mut() {
                    t.encoder_ms += stage.elapsed().as_secs_f64() * 1000.0;
                }
                probabilities
            } else {
                let hidden = self.encode_windows(&windows)?;
                if let Some(t) = timings.as_deref_mut() {
                    self.device
                        .synchronize()
                        .map_err(|_| Error::InferenceFailed)?;
                    t.encoder_ms += stage.elapsed().as_secs_f64() * 1000.0;
                }
                let stage = Instant::now();
                let probabilities = (|| -> candle_core::Result<Vec<Vec<Vec<f32>>>> {
                    let classifier = self.classifier.as_ref().expect("candle classifier");
                    softmax_last_dim(&classifier.forward(&hidden)?.to_dtype(DType::F32)?)?.to_vec3()
                })()
                .map_err(|_| Error::InferenceFailed)?;
                if let Some(t) = timings.as_deref_mut() {
                    t.classifier_ms += stage.elapsed().as_secs_f64() * 1000.0;
                }
                probabilities
            };
            let stage = Instant::now();
            for (row, range) in batch.iter().enumerate() {
                for (index, &(a, b)) in offsets[range.clone()].iter().enumerate() {
                    if a < b {
                        let score = probabilities[row][index + 1][1];
                        if !score.is_finite() || !(0.0..=1.0).contains(&score) {
                            return Err(Error::InferenceFailed);
                        }
                        spans.push(Span {
                            start: a,
                            end: b,
                            score,
                        });
                    }
                }
            }
            if let Some(t) = timings.as_deref_mut() {
                t.offsets_ms += stage.elapsed().as_secs_f64() * 1000.0;
            }
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err(Error::Timeout);
        }
        Ok(spans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_preserve_overlap_and_tail_boundaries() {
        assert!(window_ranges(0).is_empty());
        assert_eq!(window_ranges(510), vec![0..510]);
        assert_eq!(window_ranges(511), vec![0..510, 382..511]);
        assert_eq!(window_ranges(893), vec![0..510, 382..892, 764..893]);
    }
    #[test]
    fn invalid_batch_is_rejected_before_artifact_loading() {
        assert!(matches!(
            ModelDetector::load_policy_with_execution(
                "missing",
                Policy::OffensiveSpans,
                ExecutionOptions {
                    backend: Backend::Cpu,
                    coreml_dir: None,
                    compute_units: crate::ComputeUnits::All,
                    fp16: false,
                    window_batch: 0
                }
            ),
            Err(Error::InvalidOptions)
        ));
    }
}
