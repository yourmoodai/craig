//! Consented local records. Text is never admitted by metrics consent alone.
use crate::{DAY_MS, ReviewResult, now_ms, store::fingerprint};
use rand::RngCore;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

pub const RETENTION_MS: i64 = 7 * DAY_MS;
pub const MAX_STORE_BYTES: u64 = 268_435_456; // 256 MiB
pub const MAX_EVENT_BYTES: usize = 33_554_432; // 32 MiB

fn db<T>(value: rusqlite::Result<T>) -> ReviewResult<T> {
    value.map_err(|_| "trace_store_unavailable".into())
}
pub fn random_id() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn private_directory(path: &Path) -> ReviewResult<()> {
    if !path.is_absolute() {
        return Err("invalid_state_path".into());
    }
    let mut current = PathBuf::new();
    for part in path.components() {
        if matches!(part, std::path::Component::ParentDir) {
            return Err("unsafe_state_path".into());
        }
        current.push(part);
        if let Ok(meta) = fs::symlink_metadata(&current)
            && (meta.file_type().is_symlink() || !meta.is_dir())
        {
            return Err("unsafe_state_path".into());
        }
    }
    if !path.exists() {
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            private_directory(parent)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder
                .mode(0o700)
                .create(path)
                .map_err(|_| "trace_store_unavailable")?;
        }
        #[cfg(windows)]
        fs::create_dir(path).map_err(|_| "trace_store_unavailable")?;
    }
    crate::store::private_dir(path)
}
fn regular(path: &Path) -> ReviewResult<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err("unsafe_state_path".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() != 1 {
                return Err("unsafe_state_path".into());
            }
        }
    }
    Ok(())
}
pub fn state_directory() -> ReviewResult<PathBuf> {
    let root = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("craig/state"))
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
            .map(|p| p.join("craig"))
    }
    .ok_or("state_unavailable")?;
    let profile = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|p| PathBuf::from(p).join(".claude"))
        })
        .ok_or("profile_unavailable")?;
    if !profile.is_absolute() {
        return Err("invalid_profile_path".into());
    }
    let canonical = profile.canonicalize().unwrap_or(profile);
    Ok(root
        .join("profiles")
        .join(fingerprint(canonical.as_os_str().as_encoded_bytes())))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub detected: Vec<craig::Span>,
    pub applied: Vec<craig::Span>,
    pub threshold: f32,
    pub strategy: String,
    pub marker: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trace {
    pub schema: u32,
    pub offset_unit: String,
    pub id: String,
    pub timestamp_ms: i64,
    pub release: String,
    pub release_manifest_sha256: Option<String>,
    pub model_fingerprint: Option<String>,
    pub policy: String,
    pub detector: String,
    pub elapsed_ms: f64,
    pub outcome: String,
    pub disposition: String,
    pub code: Option<String>,
    pub decision: Option<Decision>,
    pub original: Option<String>,
    pub output: Option<String>,
}
impl Trace {
    fn validate(&self) -> ReviewResult<()> {
        if self.schema != 1
            || self.offset_unit != "utf8"
            || self.id.len() != 64
            || !self.id.bytes().all(|b| b.is_ascii_hexdigit())
            || self.release != env!("CARGO_PKG_VERSION")
            || self
                .release_manifest_sha256
                .as_ref()
                .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            || self
                .model_fingerprint
                .as_ref()
                .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            || !["bert", "lexical", "unavailable"].contains(&self.detector.as_str())
            || !["offensive_spans", "directed_abuse", "unavailable"].contains(&self.policy.as_str())
            || self.code.as_ref().is_some_and(|s| {
                ![
                    "invalid_input",
                    "invalid_span",
                    "invalid_options",
                    "protected_span_detected",
                    "model_unavailable",
                    "model_incompatible",
                    "inference_failed",
                    "processing_timeout",
                    "busy",
                    "policy_unavailable",
                    "adapter_failed",
                ]
                .contains(&s.as_str())
            })
            || !self.elapsed_ms.is_finite()
            || self.elapsed_ms < 0.0
            || !["rewritten", "unchanged", "abstained", "failed", "cancelled"]
                .contains(&self.outcome.as_str())
            || ![
                "processed",
                "blocked",
                "cancelled",
                "forwarded",
                "forwarding_unknown",
            ]
            .contains(&self.disposition.as_str())
        {
            return Err("invalid_trace".into());
        }
        if let Some(original) = &self.original {
            if original.len() > craig::MAX_TEXT_BYTES {
                return Err("invalid_trace".into());
            }
            if let Some(decision) = &self.decision {
                let strategy =
                    craig::ReplacementMode::new(Some(&decision.strategy), decision.marker.clone())
                        .map_err(|_| "invalid_trace")?;
                if let Some(output) = &self.output {
                    let result = craig::apply_replacements(
                        original,
                        &decision.detected,
                        decision.threshold,
                        &strategy,
                    )
                    .map_err(|_| "invalid_trace")?;
                    if &result.text != output || result.spans != decision.applied {
                        return Err("invalid_trace".into());
                    }
                } else if !decision.applied.is_empty() {
                    return Err("invalid_trace".into());
                }
            }
        }
        Ok(())
    }
    fn without_text(mut self) -> Self {
        self.original = None;
        self.output = None;
        if let Some(decision) = self.decision.as_mut() {
            decision.marker = None;
        }
        self
    }
}

pub struct TraceStore {
    pub directory: PathBuf,
    pub(crate) connection: Connection,
    _lock: fs::File,
}
impl TraceStore {
    pub fn open(directory: &Path) -> ReviewResult<Self> {
        private_directory(directory)?;
        let lock_path = directory.join("traces.lock");
        regular(&lock_path)?;
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(lock_path)
            .map_err(|_| "trace_store_unavailable")?;
        let started = std::time::Instant::now();
        while fs2::FileExt::try_lock_exclusive(&lock).is_err() {
            if started.elapsed() > Duration::from_millis(200) {
                return Err("trace_store_busy".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let path = directory.join("traces.sqlite3");
        for name in [
            "traces.sqlite3",
            "traces.sqlite3-journal",
            "traces.sqlite3-wal",
            "traces.sqlite3-shm",
        ] {
            regular(&directory.join(name))?;
        }
        let connection = db(Connection::open(&path))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|_| "trace_store_unavailable")?;
        }
        db(connection.busy_timeout(Duration::from_millis(200)))?;
        db(connection.execute_batch("PRAGMA secure_delete=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA max_page_count=16384;
            CREATE TABLE IF NOT EXISTS trace_settings(id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL, generation INTEGER NOT NULL);
            INSERT OR IGNORE INTO trace_settings VALUES(1,0,0);
            CREATE TABLE IF NOT EXISTS traces(id TEXT PRIMARY KEY, timestamp_ms INTEGER NOT NULL, outcome TEXT NOT NULL, event TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS trace_time ON traces(timestamp_ms);
            CREATE TABLE IF NOT EXISTS annotations(id TEXT PRIMARY KEY REFERENCES traces(id) ON DELETE CASCADE, body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS projection_state(id INTEGER PRIMARY KEY,revision INTEGER NOT NULL);
            INSERT OR IGNORE INTO projection_state VALUES(1,0);
            CREATE TRIGGER IF NOT EXISTS trace_insert AFTER INSERT ON traces BEGIN UPDATE projection_state SET revision=revision+1; END;
            CREATE TRIGGER IF NOT EXISTS trace_update AFTER UPDATE ON traces BEGIN UPDATE projection_state SET revision=revision+1; END;
            CREATE TRIGGER IF NOT EXISTS trace_delete AFTER DELETE ON traces BEGIN UPDATE projection_state SET revision=revision+1; END;
            CREATE TRIGGER IF NOT EXISTS annotation_insert AFTER INSERT ON annotations BEGIN UPDATE projection_state SET revision=revision+1; END;
            CREATE TRIGGER IF NOT EXISTS annotation_update AFTER UPDATE ON annotations BEGIN UPDATE projection_state SET revision=revision+1; END;
            PRAGMA foreign_keys=ON;"))?;
        let partial = directory.join("traces.parquet.partial");
        regular(&partial)?;
        if partial.exists() {
            fs::remove_file(partial).map_err(|_| "trace_store_unavailable")?;
        }
        Ok(Self {
            directory: directory.into(),
            connection,
            _lock: lock,
        })
    }
    pub fn consent(&self) -> ReviewResult<(bool, i64)> {
        db(self.connection.query_row(
            "SELECT enabled,generation FROM trace_settings WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ))
    }
    pub fn set_consent(&self, enabled: bool) -> ReviewResult<()> {
        db(self.connection.execute(
            "UPDATE trace_settings SET enabled=?1,generation=generation+1 WHERE id=1",
            [enabled],
        ))?;
        Ok(())
    }
    pub fn record(&mut self, mut trace: Trace, generation: i64) -> ReviewResult<()> {
        trace.validate()?;
        // Admission and revocation serialize on the same transaction.
        let tx = db(self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate))?;
        let (enabled, current): (bool, i64) = db(tx.query_row(
            "SELECT enabled,generation FROM trace_settings WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ))?;
        if !enabled || generation != current {
            trace = trace.without_text();
        }
        let event = serde_json::to_string(&trace).map_err(|_| "invalid_trace")?;
        if event.len() > MAX_EVENT_BYTES {
            return Err("trace_too_large".into());
        }
        db(tx.execute(
            "INSERT INTO traces VALUES(?1,?2,?3,?4)",
            params![trace.id, trace.timestamp_ms, trace.outcome, event],
        ))?;
        db(tx.commit())?;
        self.prune(now_ms())
    }
    pub fn disposition(&self, id: &str, disposition: &str) -> ReviewResult<()> {
        if !["blocked", "cancelled", "forwarded", "forwarding_unknown"].contains(&disposition) {
            return Err("invalid_disposition".into());
        }
        let mut value = self.get(id)?;
        value["disposition"] = disposition.into();
        if disposition == "cancelled" {
            value["outcome"] = "cancelled".into();
        }
        db(self.connection.execute(
            "UPDATE traces SET event=?1 WHERE id=?2",
            params![value.to_string(), id],
        ))?;
        Ok(())
    }
    pub fn get(&self, id: &str) -> ReviewResult<Value> {
        let event: String =
            db(self
                .connection
                .query_row("SELECT event FROM traces WHERE id=?1", [id], |r| r.get(0)))?;
        serde_json::from_str(&event).map_err(|_| "invalid_trace".into())
    }
    pub fn list(&self, before: i64, outcome: &str, limit: usize) -> ReviewResult<Vec<Value>> {
        let mut statement = db(self.connection.prepare("SELECT event FROM traces WHERE timestamp_ms < ?1 AND (?2='' OR outcome=?2) ORDER BY timestamp_ms DESC,id LIMIT ?3"))?;
        let rows = db(
            statement.query_map(params![before, outcome, limit.min(100) as i64], |r| {
                r.get::<_, String>(0)
            }),
        )?;
        rows.map(|r| serde_json::from_str(&db(r)?).map_err(|_| "invalid_trace".into()))
            .collect()
    }
    pub fn annotate(&self, id: &str, body: Value) -> ReviewResult<()> {
        self.get(id)?;
        let object = body.as_object().ok_or("invalid_annotation")?;
        if object.len() != 4
            || !["profanity", "targeted_abuse", "preservation"]
                .iter()
                .all(|k| {
                    object.get(*k).and_then(Value::as_str).is_some_and(|v| {
                        ["pass", "fail", "unresolved", "not_applicable"].contains(&v)
                    })
                })
            || !object
                .get("note")
                .and_then(Value::as_str)
                .is_some_and(|s| s.len() <= 4096)
        {
            return Err("invalid_annotation".into());
        }
        if !self.consent()?.0 {
            return Err("trace_consent_required".into());
        }
        db(self.connection.execute("INSERT INTO annotations VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET body=excluded.body",params![id,body.to_string()]))?;
        Ok(())
    }
    pub fn annotation(&self, id: &str) -> ReviewResult<Value> {
        use rusqlite::OptionalExtension;
        let body: Option<String> = db(self
            .connection
            .query_row("SELECT body FROM annotations WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional())?;
        body.map(|b| serde_json::from_str(&b).map_err(|_| "invalid_annotation".into()))
            .unwrap_or(Ok(Value::Null))
    }
    fn clear_parquet(&self) -> ReviewResult<()> {
        for name in [
            "traces.parquet",
            "traces.parquet.partial",
            "traces.parquet.revision",
        ] {
            let path = self.directory.join(name);
            regular(&path)?;
            if path.exists() {
                fs::remove_file(path).map_err(|_| "trace_store_unavailable")?;
            }
        }
        Ok(())
    }
    pub fn prune(&self, now: i64) -> ReviewResult<()> {
        let removed = db(self.connection.execute(
            "DELETE FROM traces WHERE timestamp_ms < ?1",
            [now - RETENTION_MS],
        ))?;
        let size:i64 = db(self.connection.query_row("SELECT COALESCE(SUM(length(CAST(event AS BLOB))),0)+(SELECT COALESCE(SUM(length(CAST(body AS BLOB))),0) FROM annotations) FROM traces",[],|r|r.get(0)))?;
        // Reserve room for SQLite pages, annotations, and the Parquet snapshot.
        let over = size as u64 > MAX_STORE_BYTES / 8;
        if over {
            db(self.connection.execute("DELETE FROM traces WHERE id IN (SELECT id FROM traces ORDER BY timestamp_ms LIMIT MAX(1,(SELECT COUNT(*)/4 FROM traces)))",[]))?;
        }
        if removed > 0 || over {
            self.clear_parquet()?;
            db(self.connection.execute_batch("VACUUM"))?;
        }
        if over {
            return self.prune(now);
        }
        Ok(())
    }
    pub fn delete(&self) -> ReviewResult<()> {
        self.set_consent(false)?;
        db(self
            .connection
            .execute_batch("DELETE FROM traces; DELETE FROM annotations; VACUUM;"))?;
        self.clear_parquet()
    }
    pub fn status(&self) -> ReviewResult<Value> {
        let (enabled, generation) = self.consent()?;
        let count: i64 = db(self
            .connection
            .query_row("SELECT COUNT(*) FROM traces", [], |r| r.get(0)))?;
        Ok(
            json!({"schema":1,"full_text":enabled,"generation":generation,"events":count,"retention_days":7,"max_bytes":MAX_STORE_BYTES,"remote_telemetry":false}),
        )
    }
}

pub struct TraceContext {
    pub model_fingerprint: String,
    pub policy: String,
    pub detector: String,
}
pub fn record_failure(code: &str, cancelled: bool) -> ReviewResult<String> {
    let directory = state_directory()?;
    let mut store = TraceStore::open(&directory)?;
    let id = random_id();
    store.record(
        Trace {
            schema: 1,
            offset_unit: "utf8".into(),
            id: id.clone(),
            timestamp_ms: now_ms(),
            release: env!("CARGO_PKG_VERSION").into(),
            release_manifest_sha256: std::env::var("CRAIG_RELEASE_MANIFEST_SHA256").ok(),
            model_fingerprint: None,
            policy: "unavailable".into(),
            detector: "unavailable".into(),
            elapsed_ms: 0.0,
            outcome: if cancelled { "cancelled" } else { "failed" }.into(),
            disposition: if cancelled { "cancelled" } else { "blocked" }.into(),
            code: Some(code.into()),
            decision: None,
            original: None,
            output: None,
        },
        -1,
    )?;
    Ok(id)
}
pub fn rewrite_traced(
    model: &mut dyn craig::Detector,
    text: &str,
    threshold: f32,
    strategy: &craig::ReplacementMode,
    context: &TraceContext,
    assess: bool,
) -> Value {
    let start = std::time::Instant::now();
    let directory = state_directory();
    let consent = directory
        .as_ref()
        .ok()
        .and_then(|p| TraceStore::open(p).ok())
        .and_then(|store| store.consent().ok());
    let detected = model.detect(text);
    let result = detected
        .as_ref()
        .map_err(|e| e.to_string())
        .and_then(|spans| {
            craig::apply_replacements(text, spans, threshold, strategy).map_err(|e| e.to_string())
        });
    let (outcome, output, code, applied) = match &result {
        Ok(r) => (
            if r.replacement_count == 0 {
                "unchanged"
            } else {
                "rewritten"
            },
            Some(r.text.clone()),
            None,
            r.spans.clone(),
        ),
        Err(code) => (
            if code == "protected_span_detected" {
                "abstained"
            } else {
                "failed"
            },
            None,
            Some(code.clone()),
            Vec::new(),
        ),
    };
    let mut answer = match &result {
        Ok(r) => {
            json!({"version":2,"status":outcome,"text":r.text,"replacement_count":r.replacement_count})
        }
        Err(code) => json!({"version":2,"status":"error","code":code}),
    };
    if assess {
        let removed: usize = applied
            .iter()
            .map(|s| text[s.start..s.end].chars().count())
            .sum();
        let characters = text.chars().count();
        let residual = output
            .as_ref()
            .and_then(|t| model.detect(t).ok())
            .and_then(|s| s.iter().map(|s| s.score).reduce(f32::max));
        answer["assessment"] = json!({"input_characters":characters,"removed_characters":removed,"replacement_count":applied.len(),"removed_proportion":if characters==0 {0.0} else {removed as f64/characters as f64},"detector_confidence":detected.as_ref().ok().and_then(|s|s.iter().map(|s|s.score).reduce(f32::max)),"residual_confidence":residual,"protected_abstention":outcome=="abstained"});
    }
    let id = random_id();
    let full = consent.is_some_and(|c| c.0);
    let trace = Trace {
        schema: 1,
        offset_unit: "utf8".into(),
        id: id.clone(),
        timestamp_ms: now_ms(),
        release: env!("CARGO_PKG_VERSION").into(),
        release_manifest_sha256: std::env::var("CRAIG_RELEASE_MANIFEST_SHA256").ok(),
        model_fingerprint: Some(context.model_fingerprint.clone()),
        policy: context.policy.clone(),
        detector: context.detector.clone(),
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
        outcome: outcome.into(),
        disposition: if result.is_ok() {
            "processed"
        } else {
            "blocked"
        }
        .into(),
        code,
        decision: detected.ok().map(|spans| Decision {
            detected: spans,
            applied,
            threshold,
            strategy: strategy.name().into(),
            marker: strategy.marker().map(str::to_owned),
        }),
        original: full.then(|| text.to_owned()),
        output: if full { output } else { None },
    };
    let saved = directory
        .and_then(|p| TraceStore::open(&p))
        .and_then(|mut store| store.record(trace, consent.map_or(-1, |c| c.1)));
    if saved.is_ok() {
        answer["trace_id"] = id.into();
    } else {
        answer["trace_warning"] = "trace_not_recorded".into();
    }
    answer
}
