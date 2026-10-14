use super::*;
use middle_finger::decision::DecisionDetector;

#[napi(object)]
pub struct ExperimentalOptions {
    #[napi(ts_type = "'lexical' | 'crf'")]
    pub kind: String,
    pub artifact_dir: String,
}
#[napi]
pub struct ExperimentalDetector {
    model: Arc<Mutex<DecisionDetector>>,
    threshold: f32,
    busy: Arc<AtomicBool>,
}
pub struct LoadExperimental {
    options: ExperimentalOptions,
}
#[napi]
impl Task for LoadExperimental {
    type Output = DecisionDetector;
    type JsValue = ExperimentalDetector;
    fn compute(&mut self) -> Result<Self::Output> {
        DecisionDetector::load(&self.options.artifact_dir, &self.options.kind).map_err(err)
    }
    fn resolve(&mut self, _: Env, model: Self::Output) -> Result<Self::JsValue> {
        Ok(ExperimentalDetector {
            threshold: model.threshold(),
            model: Arc::new(Mutex::new(model)),
            busy: Arc::new(AtomicBool::new(false)),
        })
    }
}
#[napi]
pub fn load_experimental_detector(options: ExperimentalOptions) -> AsyncTask<LoadExperimental> {
    AsyncTask::new(LoadExperimental { options })
}
pub struct ExperimentalRewrite {
    model: Arc<Mutex<DecisionDetector>>,
    text: String,
    threshold: f32,
    marker: core::FixedMarker,
    _permit: Permit,
}
#[napi]
impl Task for ExperimentalRewrite {
    type Output = core::RewriteResult;
    type JsValue = RewriteResult;
    fn compute(&mut self) -> Result<Self::Output> {
        let mut model = self.model.try_lock().map_err(|_| err(core::Error::Busy))?;
        core::rewrite(&mut *model, &self.text, self.threshold, &self.marker).map_err(err)
    }
    fn resolve(&mut self, _: Env, result: Self::Output) -> Result<Self::JsValue> {
        output(&self.text, result)
    }
}
#[napi]
impl ExperimentalDetector {
    #[napi]
    pub fn rewrite(
        &self,
        text: Utf16String,
        options: Option<ReplacementOptions>,
    ) -> Result<AsyncTask<ExperimentalRewrite>> {
        Ok(AsyncTask::new(ExperimentalRewrite {
            model: self.model.clone(),
            text: valid_text(text)?,
            threshold: self.threshold,
            marker: marker(options)?,
            _permit: Permit::acquire(&self.busy)?,
        }))
    }
}
