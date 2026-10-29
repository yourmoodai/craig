pub mod decision;
use napi::{Task, bindgen_prelude::*};
use napi_derive::napi;
use savior as core;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Permit(Arc<AtomicBool>);
impl Permit {
    fn acquire(busy: &Arc<AtomicBool>) -> Result<Self> {
        busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| err(core::Error::Busy))?;
        Ok(Self(busy.clone()))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn err(error: core::Error) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}
#[napi(object)]
pub struct Options {
    pub model_dir: String,
    #[napi(ts_type = "'bert' | 'lexical'")]
    pub detector: Option<String>,
    pub threshold: Option<f64>,
    #[napi(ts_type = "'offensive_spans' | 'directed_abuse'")]
    pub policy: Option<String>,
    #[napi(ts_type = "'cpu' | 'metal' | 'core_ml'")]
    pub backend: Option<String>,
    #[napi(ts_type = "'float32' | 'float16'")]
    pub precision: Option<String>,
    pub window_batch: Option<u32>,
    pub coreml_dir: Option<String>,
    #[napi(ts_type = "'all' | 'cpu_only' | 'cpu_gpu' | 'cpu_neural_engine'")]
    pub compute_units: Option<String>,
}
#[napi(object)]
pub struct ReplacementOptions {
    #[napi(ts_type = "'fixed_phrase' | 'omit'")]
    pub strategy: Option<String>,
    pub marker: Option<String>,
}
#[napi(object)]
pub struct Span {
    pub start: f64,
    pub end: f64,
    pub score: f64,
}
#[napi(object)]
pub struct DetectionResult {
    pub spans: Vec<Span>,
    pub offset_unit: String,
}
#[napi(object)]
pub struct RewriteResult {
    pub text: String,
    pub replacement_count: u32,
    pub spans: Vec<Span>,
    pub offset_unit: String,
}

#[napi(object)]
pub struct LanguageAssessment {
    pub input_characters: u32,
    pub removed_characters: u32,
    pub replacement_count: u32,
    pub removed_proportion: f64,
    pub detector_confidence: Option<f64>,
    pub residual_confidence: Option<f64>,
    pub protected_abstention: bool,
}
#[napi(object)]
pub struct AssessedRewrite {
    pub text: Option<String>,
    pub assessment: LanguageAssessment,
}
pub struct Assess {
    model: Arc<Mutex<core::RuntimeDetector>>,
    text: String,
    threshold: f32,
    strategy: core::ReplacementMode,
    _permit: Permit,
}
#[napi]
impl Task for Assess {
    type Output = (
        savior_review::assessment::LanguageAssessment,
        Option<String>,
    );
    type JsValue = AssessedRewrite;
    fn compute(&mut self) -> Result<Self::Output> {
        let mut model = self.model.try_lock().map_err(|_| err(core::Error::Busy))?;
        savior_review::assessment::assess(&mut *model, &self.text, self.threshold, &self.strategy)
            .map_err(|code| Error::new(Status::GenericFailure, code))
    }
    fn resolve(&mut self, _: Env, (assessment, text): Self::Output) -> Result<Self::JsValue> {
        Ok(AssessedRewrite {
            text,
            assessment: LanguageAssessment {
                input_characters: assessment.input_characters as u32,
                removed_characters: assessment.removed_characters as u32,
                replacement_count: assessment.replacement_count as u32,
                removed_proportion: assessment.removed_proportion,
                detector_confidence: assessment.detector_confidence.map(f64::from),
                residual_confidence: assessment.residual_confidence.map(f64::from),
                protected_abstention: assessment.protected_abstention,
            },
        })
    }
}

fn output_spans(text: &str, spans: Vec<core::Span>) -> Result<Vec<Span>> {
    // Normalized spans are ordered and disjoint; scan source characters once.
    let mut cursor = 0;
    let mut units = 0;
    spans
        .into_iter()
        .map(|s| {
            let gap = text
                .get(cursor..s.start)
                .ok_or_else(|| err(core::Error::InvalidSpan))?;
            let matched = text
                .get(s.start..s.end)
                .ok_or_else(|| err(core::Error::InvalidSpan))?;
            units += gap.encode_utf16().count();
            let start = units;
            units += matched.encode_utf16().count();
            cursor = s.end;
            Ok(Span {
                start: start as f64,
                end: units as f64,
                score: s.score as f64,
            })
        })
        .collect()
}
fn output(text: &str, result: core::RewriteResult) -> Result<RewriteResult> {
    Ok(RewriteResult {
        spans: output_spans(text, result.spans)?,
        text: result.text,
        replacement_count: result.replacement_count as u32,
        offset_unit: "utf16".into(),
    })
}
fn replacement_mode(options: Option<ReplacementOptions>) -> Result<core::ReplacementMode> {
    let (strategy, marker) = options
        .map(|options| (options.strategy, options.marker))
        .unwrap_or((None, None));
    core::ReplacementMode::new(strategy.as_deref(), marker).map_err(err)
}

#[napi]
pub fn apply_replacements(
    text: Utf16String,
    spans: Vec<Span>,
    options: Option<ReplacementOptions>,
) -> Result<RewriteResult> {
    let text = valid_text(text)?;
    if spans.len() > core::MAX_TEXT_BYTES {
        return Err(err(core::Error::InvalidSpan));
    }
    let spans: Vec<_> = spans
        .into_iter()
        .map(|s| {
            if !s.score.is_finite() || !(0.0..=1.0).contains(&s.score) {
                return Err(err(core::Error::InvalidSpan));
            }
            Ok(core::Span {
                start: core::utf16_to_byte(&text, offset(s.start)?).map_err(err)?,
                end: core::utf16_to_byte(&text, offset(s.end)?).map_err(err)?,
                score: s.score as f32,
            })
        })
        .collect::<Result<_>>()?;
    let result =
        core::apply_replacements(&text, &spans, 0.0, &replacement_mode(options)?).map_err(err)?;
    output(&text, result)
}

fn valid_text(text: Utf16String) -> Result<String> {
    let text = String::from_utf16(&text).map_err(|_| err(core::Error::InvalidInput))?;
    core::validate_text(&text).map_err(err)?;
    Ok(text)
}
fn offset(value: f64) -> Result<u32> {
    if !value.is_finite() || value < 0.0 || value > u32::MAX as f64 || value.fract() != 0.0 {
        return Err(err(core::Error::InvalidSpan));
    }
    Ok(value as u32)
}

#[napi]
pub struct Detector {
    model: Arc<Mutex<core::RuntimeDetector>>,
    threshold: f32,
    busy: Arc<AtomicBool>,
    trace_context: Arc<savior_review::traces::TraceContext>,
}
pub struct Load {
    options: Options,
}
#[napi]
impl Task for Load {
    type Output = core::RuntimeDetector;
    type JsValue = Detector;
    fn compute(&mut self) -> Result<Self::Output> {
        let threshold = self.options.threshold.unwrap_or(0.5);
        if !threshold.is_finite() || threshold <= 0.0 || threshold > 1.0 {
            return Err(err(core::Error::InvalidOptions));
        }
        let policy = self
            .options
            .policy
            .as_deref()
            .unwrap_or("offensive_spans")
            .parse()
            .map_err(err)?;
        let fp16 = match self.options.precision.as_deref().unwrap_or("float32") {
            "float32" => false,
            "float16" => true,
            _ => return Err(err(core::Error::InvalidOptions)),
        };
        let backend: core::Backend = self
            .options
            .backend
            .as_deref()
            .unwrap_or("cpu")
            .parse()
            .map_err(err)?;
        let execution = core::ExecutionOptions {
            backend,
            fp16,
            window_batch: self.options.window_batch.unwrap_or(1) as usize,
            coreml_dir: self.options.coreml_dir.as_ref().map(Into::into),
            compute_units: self
                .options
                .compute_units
                .as_deref()
                .unwrap_or(if backend == core::Backend::CoreMl {
                    "cpu_gpu"
                } else {
                    "all"
                })
                .parse()
                .map_err(err)?,
        };
        core::RuntimeDetector::load(
            &self.options.model_dir,
            self.options.detector.as_deref().unwrap_or("bert"),
            policy,
            execution,
        )
        .map_err(err)
    }
    fn resolve(&mut self, _: Env, model: Self::Output) -> Result<Detector> {
        let threshold = self
            .options
            .threshold
            .map(|t| t as f32)
            .unwrap_or(model.threshold());
        let model_fingerprint = match &model {
            core::RuntimeDetector::Bert(model) => model.fingerprint().to_owned(),
            core::RuntimeDetector::Lexical(model) => savior_review::store::fingerprint(
                &serde_json::to_vec(&model.artifact)
                    .map_err(|_| err(core::Error::ModelIncompatible))?,
            ),
        };
        Ok(Detector {
            model: Arc::new(Mutex::new(model)),
            threshold,
            busy: Arc::new(AtomicBool::new(false)),
            trace_context: Arc::new(savior_review::traces::TraceContext {
                model_fingerprint,
                detector: self
                    .options
                    .detector
                    .clone()
                    .unwrap_or_else(|| "bert".into()),
                policy: self
                    .options
                    .policy
                    .clone()
                    .unwrap_or_else(|| "offensive_spans".into()),
            }),
        })
    }
}

pub struct TracedRewrite {
    model: Arc<Mutex<core::RuntimeDetector>>,
    context: Arc<savior_review::traces::TraceContext>,
    text: String,
    threshold: f32,
    strategy: core::ReplacementMode,
    assess: bool,
    _permit: Permit,
}
#[napi]
impl Task for TracedRewrite {
    type Output = String;
    type JsValue = String;
    fn compute(&mut self) -> Result<String> {
        let mut model = self.model.try_lock().map_err(|_| err(core::Error::Busy))?;
        Ok(savior_review::traces::rewrite_traced(
            &mut *model,
            &self.text,
            self.threshold,
            &self.strategy,
            &self.context,
            self.assess,
        )
        .to_string())
    }
    fn resolve(&mut self, _: Env, value: String) -> Result<String> {
        Ok(value)
    }
}
#[napi]
pub fn load_detector(options: Options) -> AsyncTask<Load> {
    AsyncTask::new(Load { options })
}

pub struct Detect {
    model: Arc<Mutex<core::RuntimeDetector>>,
    text: String,
    threshold: f32,
    _permit: Permit,
}
#[napi]
impl Task for Detect {
    type Output = Vec<core::Span>;
    type JsValue = DetectionResult;
    fn compute(&mut self) -> Result<Self::Output> {
        use core::Detector;
        let mut model = self.model.try_lock().map_err(|_| err(core::Error::Busy))?;
        let spans = model.detect(&self.text).map_err(err)?;
        core::normalize_spans(&self.text, &spans, self.threshold).map_err(err)
    }
    fn resolve(&mut self, _: Env, spans: Self::Output) -> Result<DetectionResult> {
        Ok(DetectionResult {
            spans: output_spans(&self.text, spans)?,
            offset_unit: "utf16".into(),
        })
    }
}
pub struct Rewrite {
    model: Arc<Mutex<core::RuntimeDetector>>,
    text: String,
    threshold: f32,
    strategy: core::ReplacementMode,
    _permit: Permit,
}
#[napi]
impl Task for Rewrite {
    type Output = core::RewriteResult;
    type JsValue = RewriteResult;
    fn compute(&mut self) -> Result<Self::Output> {
        let mut model = self.model.try_lock().map_err(|_| err(core::Error::Busy))?;
        core::rewrite(&mut *model, &self.text, self.threshold, &self.strategy).map_err(err)
    }
    fn resolve(&mut self, _: Env, result: Self::Output) -> Result<RewriteResult> {
        output(&self.text, result)
    }
}
#[napi]
impl Detector {
    #[napi]
    pub fn rewrite_traced(
        &self,
        text: Utf16String,
        options: Option<ReplacementOptions>,
        assess: Option<bool>,
    ) -> Result<AsyncTask<TracedRewrite>> {
        Ok(AsyncTask::new(TracedRewrite {
            model: self.model.clone(),
            context: self.trace_context.clone(),
            text: valid_text(text)?,
            threshold: self.threshold,
            strategy: replacement_mode(options)?,
            assess: assess.unwrap_or(false),
            _permit: Permit::acquire(&self.busy)?,
        }))
    }
    #[napi]
    pub fn assess(
        &self,
        text: Utf16String,
        options: Option<ReplacementOptions>,
    ) -> Result<AsyncTask<Assess>> {
        Ok(AsyncTask::new(Assess {
            model: self.model.clone(),
            text: valid_text(text)?,
            threshold: self.threshold,
            strategy: replacement_mode(options)?,
            _permit: Permit::acquire(&self.busy)?,
        }))
    }
    #[napi]
    pub fn detect(&self, text: Utf16String) -> Result<AsyncTask<Detect>> {
        let text = valid_text(text)?;
        Ok(AsyncTask::new(Detect {
            model: self.model.clone(),
            text,
            threshold: self.threshold,
            _permit: Permit::acquire(&self.busy)?,
        }))
    }
    #[napi]
    pub fn rewrite(
        &self,
        text: Utf16String,
        options: Option<ReplacementOptions>,
    ) -> Result<AsyncTask<Rewrite>> {
        let text = valid_text(text)?;
        Ok(AsyncTask::new(Rewrite {
            model: self.model.clone(),
            text,
            threshold: self.threshold,
            strategy: replacement_mode(options)?,
            _permit: Permit::acquire(&self.busy)?,
        }))
    }
}
