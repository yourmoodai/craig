use crate::{
    ReviewResult,
    assessment::{CONTEXTUAL_RUBRIC, LanguageAssessment, assess},
    history::{ClaudeHistory, HistorySource, valid_id, window},
    judge::Judge,
    policy::POLICY_RECORD,
    store::{
        Consent, JudgeStatus, Metric, MetricsStore, ObservationSource, SqliteMetrics, fingerprint,
    },
};
use craig::{Policy, RuntimeDetector};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub id: String,
    pub session: String,
    pub generation: u64,
    pub timestamp_ms: i64,
    pub source: ObservationSource,
    pub original: String,
    pub replacement: Option<String>,
    pub assessment: Option<LanguageAssessment>,
    #[serde(default)]
    pub context: Vec<String>,
    #[serde(default)]
    pub authorship_verified: bool,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status,
    Export,
    Policy,
    Consent { live: bool, history: bool },
    Delete,
    Cancel { session: Option<String> },
    HistoryIndex { days: u8 },
    Observe { observation: Observation },
    Shutdown,
}
#[derive(Clone)]
struct Job {
    observation: Observation,
    id: String,
    session: String,
    metric: Metric,
    cancel: Arc<AtomicBool>,
    judge: bool,
}
pub struct Worker {
    profile: PathBuf,
    store: Mutex<SqliteMetrics>,
    queue: Mutex<VecDeque<Job>>,
    wake: Condvar,
    active: Mutex<Option<(String, Arc<AtomicBool>)>>,
    pub stopping: AtomicBool,
    model: Option<PathBuf>,
    judge_config: Option<PathBuf>,
    policy: Policy,
    detector_kind: String,
    threshold: Option<f32>,
    replacement: craig::ReplacementMode,
    model_fingerprint: String,
    runtime_fingerprint: String,
}
fn lock<T>(mutex: &Mutex<T>) -> ReviewResult<std::sync::MutexGuard<'_, T>> {
    mutex.lock().map_err(|_| "review_unavailable".into())
}
impl Worker {
    pub fn new(profile: &Path) -> ReviewResult<Arc<Self>> {
        let model = std::env::var_os("CRAIG_MODEL_DIR").map(PathBuf::from);
        let policy = std::env::var("CRAIG_POLICY")
            .unwrap_or_else(|_| "offensive_spans".into())
            .parse()
            .map_err(|_| "invalid_policy")?;
        let threshold = std::env::var("CRAIG_THRESHOLD")
            .ok()
            .map(|s| s.parse::<f32>().map_err(|_| "invalid_threshold"))
            .transpose()?;
        if threshold.is_some_and(|t| !t.is_finite() || t <= 0.0 || t > 1.0) {
            return Err("invalid_threshold".into());
        }
        let replacement_strategy =
            std::env::var("CRAIG_REPLACEMENT_STRATEGY").unwrap_or_else(|_| "fixed_phrase".into());
        let replacement = craig::ReplacementMode::new(
            Some(&replacement_strategy),
            std::env::var("CRAIG_REPLACEMENT").ok(),
        )
        .map_err(|_| "invalid_options")?;
        let detector_kind = std::env::var("CRAIG_DETECTOR").unwrap_or_else(|_| "bert".into());
        if !["bert", "lexical"].contains(&detector_kind.as_str()) {
            return Err("invalid_options".into());
        }
        let head = model
            .as_ref()
            .and_then(|p| {
                std::fs::read(p.join(if detector_kind == "lexical" {
                    "decision.json"
                } else {
                    "manifest.json"
                }))
                .ok()
            })
            .map(|b| fingerprint(&b));
        let model_fingerprint=fingerprint(json!({"detector":detector_kind,"base":craig::MODEL_REVISION,"head":head,"policy":policy,"threshold":threshold}).to_string().as_bytes());
        let mut store = SqliteMetrics::open(profile)?;
        store.expire(crate::now_ms())?;
        store.cancel_pending(None)?;
        let mut settings: Vec<String> = [
            "CRAIG_MODEL_DIR",
            "CRAIG_DETECTOR",
            "CRAIG_POLICY",
            "CRAIG_THRESHOLD",
            "CRAIG_REPLACEMENT_STRATEGY",
            "CRAIG_REPLACEMENT",
            "CRAIG_JUDGE_CONFIG",
        ]
        .iter()
        .map(|key| std::env::var(key).unwrap_or_default())
        .collect();
        settings.push(head.unwrap_or_default());
        let runtime_fingerprint = fingerprint(
            serde_json::to_string(&settings)
                .map_err(|_| "invalid_configuration")?
                .as_bytes(),
        );
        Ok(Arc::new(Self {
            profile: profile.into(),
            store: Mutex::new(store),
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            active: Mutex::new(None),
            stopping: AtomicBool::new(false),
            model,
            judge_config: std::env::var_os("CRAIG_JUDGE_CONFIG").map(PathBuf::from),
            policy,
            detector_kind,
            threshold,
            replacement,
            model_fingerprint,
            runtime_fingerprint,
        }))
    }
    pub fn cancel(&self, session: Option<&str>) -> ReviewResult<()> {
        let mut queue = lock(&self.queue)?;
        queue.retain(|job| {
            let keep = session.is_some_and(|s| s != job.observation.session);
            if !keep {
                job.cancel.store(true, Ordering::Release);
            }
            keep
        });
        if let Some((id, token)) = &*lock(&self.active)?
            && session.is_none_or(|s| s == id)
        {
            token.store(true, Ordering::Release);
        }
        drop(queue);
        let mut store = lock(&self.store)?;
        let session = session.map(|s| store.opaque("session", s));
        store.cancel_pending(session.as_deref())?;
        self.wake.notify_all();
        Ok(())
    }
    pub fn busy(&self) -> bool {
        self.queue.lock().is_ok_and(|q| !q.is_empty())
            || self.active.lock().is_ok_and(|a| a.is_some())
    }
    pub fn handle(&self, request: Request) -> ReviewResult<Value> {
        let now = crate::now_ms();
        match request {
            Request::Policy => Ok(
                json!({"record":serde_json::from_str::<Value>(POLICY_RECORD).map_err(|_|"invalid_policy")?,"fingerprint":fingerprint(POLICY_RECORD.as_bytes()),"version":2,"policy_alignment_status":"inconclusive","judge_calibrated":false}),
            ),
            Request::Status | Request::Export => {
                let mut result = lock(&self.store)?.export(now)?;
                result["busy"] = self.busy().into();
                result["runtime_fingerprint"] = self.runtime_fingerprint.clone().into();
                result["history_capabilities"] =
                    serde_json::to_value(crate::history::capabilities())
                        .map_err(|_| "review_unavailable")?;
                Ok(result)
            }
            Request::Consent { live, history } => {
                self.cancel(None)?;
                Ok(json!({"consent":lock(&self.store)?.set_consent(live,history)?}))
            }
            Request::Delete => {
                self.cancel(None)?;
                let mut store = lock(&self.store)?;
                store.clear()?;
                Ok(json!({"consent":store.consent()?,"deleted":true}))
            }
            Request::Cancel { session } => {
                self.cancel(session.as_deref())?;
                Ok(json!({"cancelled":true}))
            }
            Request::Shutdown => {
                self.stopping.store(true, Ordering::Release);
                self.cancel(None)?;
                Ok(json!({"stopped":true}))
            }
            Request::HistoryIndex { days } => {
                if !lock(&self.store)?.consent()?.history {
                    return Err("history_not_enabled".into());
                }
                serde_json::to_value(
                    ClaudeHistory {
                        profile: &self.profile,
                    }
                    .index(now, days)?,
                )
                .map_err(|_| "history_unavailable".into())
            }
            Request::Observe { observation } => self.observe(observation, now),
        }
    }
    fn observe(&self, observation: Observation, now: i64) -> ReviewResult<Value> {
        if !valid_id(&observation.id) || !valid_id(&observation.session) {
            return Err("invalid_observation".into());
        }
        craig::validate_text(&observation.original).map_err(|_| "invalid_observation")?;
        if observation.context.len() > 4
            || observation.context.iter().map(String::len).sum::<usize>() > 16_384
        {
            return Err("context_too_large".into());
        } // 16 KiB
        if let Some(text) = &observation.replacement {
            craig::validate_text(text).map_err(|_| "invalid_observation")?;
        }
        if observation.source == ObservationSource::History
            && !window(now, 5, observation.timestamp_ms)?
        {
            return Err("outside_history_window".into());
        }
        if observation.timestamp_ms > now || observation.timestamp_ms < now - 30 * crate::DAY_MS {
            return Err("invalid_metric_time".into());
        }
        let mut store = lock(&self.store)?;
        let consent = store.consent()?;
        if !enabled(&consent, &observation) {
            return Err("review_not_enabled".into());
        }
        let id = store.opaque(
            "observation",
            &format!(
                "{:?}:{}:{}:{}:{}",
                observation.source,
                observation.session,
                observation.id,
                self.model_fingerprint,
                self.runtime_fingerprint
            ),
        );
        let session = store.opaque("session", &observation.session);
        if store.contains(&id)? && !store.retryable_history(&id)? {
            return Ok(json!({"status":"duplicate"}));
        }
        let wants_judge = if observation.source == ObservationSource::Live {
            store.sampled(&id, now)?
        } else {
            let bytes = fingerprint(id.as_bytes());
            u8::from_str_radix(&bytes[..2], 16)
                .unwrap_or(255)
                .is_multiple_of(10)
        };
        let assessment = observation.assessment.clone();
        if observation.source == ObservationSource::Live {
            let a = assessment.as_ref().ok_or("missing_assessment")?;
            if a.input_characters != observation.original.chars().count()
                || a.removed_characters > a.input_characters
                || !a.removed_proportion.is_finite()
                || !(0.0..=1.0).contains(&a.removed_proportion)
                || a.detector_confidence
                    .into_iter()
                    .chain(a.residual_confidence)
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
            {
                return Err("invalid_assessment".into());
            }
        }
        let mut metric = Metric {
            source: observation.source,
            timestamp_ms: observation.timestamp_ms,
            assessment,
            judge_status: if wants_judge {
                JudgeStatus::Pending
            } else {
                JudgeStatus::NotSampled
            },
            contextual: None,
            model_fingerprint: self.model_fingerprint.clone(),
            rubric_fingerprint: fingerprint(CONTEXTUAL_RUBRIC.as_bytes()),
            judge_fingerprint: None,
            policy_fingerprint: fingerprint(POLICY_RECORD.as_bytes()),
            marker_fingerprint: fingerprint(
                format!(
                    "{}:{}",
                    self.replacement.name(),
                    self.replacement.marker().unwrap_or_default()
                )
                .as_bytes(),
            ),
            assessment_failed: false,
            authorship_verified: observation.authorship_verified,
        };
        let needs_job = wants_judge || observation.source == ObservationSource::History;
        let mut queue = lock(&self.queue)?;
        if needs_job && queue.len() >= 4 {
            if observation.source == ObservationSource::History {
                return Ok(json!({"status":"busy"}));
            }
            metric.judge_status = JudgeStatus::Capacity;
            store.count("queue_full", now)?;
        } else if needs_job {
            queue.push_back(Job {
                observation: observation.clone(),
                id: id.clone(),
                session: session.clone(),
                metric: metric.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                judge: wants_judge,
            });
        }
        store.record(&id, &session, &metric, observation.generation, now)?;
        self.wake.notify_one();
        Ok(json!({"status":"accepted","judge_status":metric.judge_status}))
    }
    pub fn run(self: Arc<Self>) {
        let mut detector = None;
        loop {
            let job = {
                let Ok(mut queue) = self.queue.lock() else {
                    break;
                };
                while queue.is_empty() && !self.stopping.load(Ordering::Acquire) {
                    let Ok((q, _)) = self.wake.wait_timeout(queue, Duration::from_secs(1)) else {
                        return;
                    };
                    queue = q;
                }
                if self.stopping.load(Ordering::Acquire) {
                    break;
                }
                let job = queue.pop_front();
                if let Some(job) = &job
                    && let Ok(mut active) = self.active.lock()
                {
                    *active = Some((job.observation.session.clone(), job.cancel.clone()));
                }
                job
            };
            let Some(mut job) = job else { continue };
            let result = self.process(&mut job, &mut detector);
            if result.is_err() {
                job.metric.assessment_failed = job.metric.assessment.is_none();
                job.metric.judge_status = JudgeStatus::Unavailable;
            }
            if job.cancel.load(Ordering::Acquire) {
                job.metric.contextual = None;
                job.metric.judge_status = JudgeStatus::Cancelled;
            }
            if let Ok(mut store) = self.store.lock() {
                if job.cancel.load(Ordering::Acquire) {
                    job.metric.contextual = None;
                    job.metric.judge_status = JudgeStatus::Cancelled;
                    job.metric.assessment_failed = job.metric.assessment.is_none();
                }
                let _ = store.record(
                    &job.id,
                    &job.session,
                    &job.metric,
                    job.observation.generation,
                    crate::now_ms(),
                );
            }
            if let Ok(mut active) = self.active.lock() {
                *active = None;
            }
        }
    }
    fn process(&self, job: &mut Job, detector: &mut Option<RuntimeDetector>) -> ReviewResult<()> {
        if job.cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        if job.observation.source == ObservationSource::History {
            if detector.is_none() {
                *detector = Some(
                    RuntimeDetector::load(
                        self.model.as_ref().ok_or("model_not_configured")?,
                        &self.detector_kind,
                        self.policy,
                        Default::default(),
                    )
                    .map_err(|e| e.to_string())?,
                );
            }
            let model = detector.as_mut().ok_or("model_not_configured")?;
            let threshold = self.threshold.unwrap_or(model.threshold());
            let (assessment, replacement) = assess(
                model,
                &job.observation.original,
                threshold,
                &self.replacement,
            )?;
            job.metric.assessment = Some(assessment);
            job.observation.replacement = replacement;
        }
        if !job.judge || job.cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut judge = Judge::start_with_cancel(
            self.judge_config.as_ref().ok_or("judge_not_configured")?,
            job.cancel.clone(),
        )?;
        let input = json!({"original":job.observation.original,"replacement":job.observation.replacement,"context":job.observation.context,"replacement_strategy":self.replacement.name(),"marker":self.replacement.marker(),"replacement_policy":self.policy,"policy_record":serde_json::from_str::<Value>(POLICY_RECORD).map_err(|_|"invalid_policy")?});
        job.metric.contextual = Some(judge.contextual(input)?);
        job.metric.judge_fingerprint = Some(fingerprint(judge.provenance.to_string().as_bytes()));
        job.metric.judge_status = JudgeStatus::Complete;
        Ok(())
    }
}
fn enabled(consent: &Consent, observation: &Observation) -> bool {
    consent.generation == observation.generation
        && match observation.source {
            ObservationSource::Live => consent.live,
            ObservationSource::History => consent.history,
        }
}
