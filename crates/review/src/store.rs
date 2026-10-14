use crate::{
    DAY_MS, ReviewResult,
    assessment::{ContextualAssessment, LanguageAssessment},
    policy::POLICY_ID,
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Consent {
    pub live: bool,
    pub history: bool,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    Live,
    History,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JudgeStatus {
    NotSampled,
    Pending,
    Complete,
    Unavailable,
    Capacity,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metric {
    pub source: ObservationSource,
    pub timestamp_ms: i64,
    pub assessment: Option<LanguageAssessment>,
    pub judge_status: JudgeStatus,
    pub contextual: Option<ContextualAssessment>,
    pub model_fingerprint: String,
    pub rubric_fingerprint: String,
    pub judge_fingerprint: Option<String>,
    pub policy_fingerprint: String,
    pub marker_fingerprint: String,
    pub assessment_failed: bool,
    pub authorship_verified: bool,
}
pub fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn db<T>(result: rusqlite::Result<T>) -> ReviewResult<T> {
    result.map_err(|_| "metrics_unavailable".into())
}

pub fn profile_dir() -> ReviewResult<PathBuf> {
    let root = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(std::env::var_os("HOME").ok_or("profile_unavailable")?).join(".claude"),
    };
    root.canonicalize()
        .map_err(|_| "profile_unavailable".into())
}
pub fn private_dir(path: &Path) -> ReviewResult<()> {
    std::fs::create_dir_all(path).map_err(|_| "metrics_unavailable")?;
    if std::fs::symlink_metadata(path)
        .map_err(|_| "metrics_unavailable")?
        .file_type()
        .is_symlink()
    {
        return Err("unsafe_metrics_path".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "metrics_unavailable")?;
    }
    Ok(())
}

pub trait MetricsStore {
    fn record(
        &mut self,
        id: &str,
        session: &str,
        metric: &Metric,
        generation: u64,
        now: i64,
    ) -> ReviewResult<bool>;
    fn export(&mut self, now: i64) -> ReviewResult<serde_json::Value>;
}
pub struct SqliteMetrics {
    connection: Connection,
    key: Vec<u8>,
}
impl SqliteMetrics {
    pub fn open(profile: &Path) -> ReviewResult<Self> {
        let directory = profile.join("craig");
        private_dir(&directory)?;
        let path = directory.join("metrics.sqlite3");
        if path.is_symlink() {
            return Err("unsafe_metrics_path".into());
        }
        let connection = db(Connection::open(&path))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "metrics_unavailable")?;
        }
        db(connection.busy_timeout(Duration::from_secs(2)))?;
        db(connection.execute_batch("PRAGMA secure_delete=ON; PRAGMA journal_mode=DELETE;
            CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1), live INTEGER NOT NULL DEFAULT 0, history INTEGER NOT NULL DEFAULT 0, generation INTEGER NOT NULL DEFAULT 0, secret BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS metrics (id TEXT PRIMARY KEY, session TEXT NOT NULL, timestamp_ms INTEGER NOT NULL, source TEXT NOT NULL, sampled INTEGER NOT NULL, metric TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS metrics_time ON metrics(timestamp_ms);
            CREATE INDEX IF NOT EXISTS metrics_sample ON metrics(source,sampled,timestamp_ms);
            CREATE TABLE IF NOT EXISTS counters (day INTEGER NOT NULL, kind TEXT NOT NULL, count INTEGER NOT NULL, PRIMARY KEY(day,kind));"))?;
        let mut secret = [0u8; 32];
        rand::rng().fill_bytes(&mut secret);
        db(connection.execute(
            "INSERT OR IGNORE INTO settings(id,secret) VALUES(1,?1)",
            params![secret.as_slice()],
        ))?;
        let key =
            db(connection.query_row("SELECT secret FROM settings WHERE id=1", [], |r| r.get(0)))?;
        Ok(Self { connection, key })
    }
    pub fn consent(&self) -> ReviewResult<Consent> {
        db(self.connection.query_row(
            "SELECT live,history,generation FROM settings WHERE id=1",
            [],
            |r| {
                Ok(Consent {
                    live: r.get(0)?,
                    history: r.get(1)?,
                    generation: r.get::<_, i64>(2)? as u64,
                })
            },
        ))
    }
    pub fn set_consent(&mut self, live: bool, history: bool) -> ReviewResult<Consent> {
        db(self.connection.execute(
            "UPDATE settings SET live=?1,history=?2,generation=generation+1 WHERE id=1",
            params![live, history],
        ))?;
        self.consent()
    }
    pub fn opaque(&self, domain: &str, id: &str) -> String {
        let mut hmac =
            Hmac::<Sha256>::new_from_slice(&self.key).expect("hmac accepts any key length");
        hmac.update(domain.as_bytes());
        hmac.update(&[0]);
        hmac.update(id.as_bytes());
        format!("{:x}", hmac.finalize().into_bytes())
    }
    pub fn contains(&self, id: &str) -> ReviewResult<bool> {
        db(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM metrics WHERE id=?1)",
            [id],
            |r| r.get(0),
        ))
    }
    pub fn retryable_history(&self, id: &str) -> ReviewResult<bool> {
        let value: String =
            db(self
                .connection
                .query_row("SELECT metric FROM metrics WHERE id=?1", [id], |r| r.get(0)))?;
        let metric: Metric = serde_json::from_str(&value).map_err(|_| "invalid_metric")?;
        Ok(metric.source == ObservationSource::History
            && (metric.judge_status == JudgeStatus::Cancelled || metric.assessment_failed))
    }
    pub fn sampled(&self, id: &str, now: i64) -> ReviewResult<bool> {
        let digest = Sha256::digest(id.as_bytes());
        let selected = u64::from_le_bytes(digest[..8].try_into().unwrap()).is_multiple_of(10);
        let count: i64 = db(self.connection.query_row("SELECT COUNT(*) FROM metrics WHERE source='live' AND sampled=1 AND timestamp_ms>=?1 AND timestamp_ms<?2", params![now.div_euclid(DAY_MS)*DAY_MS,(now.div_euclid(DAY_MS)+1)*DAY_MS], |r|r.get(0)))?;
        Ok(selected && count < 100)
    }
    pub fn expire(&mut self, now: i64) -> ReviewResult<()> {
        db(self.connection.execute(
            "DELETE FROM metrics WHERE timestamp_ms<?1",
            [now - 30 * DAY_MS],
        ))?;
        db(self.connection.execute(
            "DELETE FROM counters WHERE day<?1",
            [(now - 30 * DAY_MS).div_euclid(DAY_MS)],
        ))?;
        Ok(())
    }
    pub fn cancel_pending(&mut self, session: Option<&str>) -> ReviewResult<()> {
        let mut statement = db(self
            .connection
            .prepare("SELECT id,metric FROM metrics WHERE (?1 IS NULL OR session=?1)"))?;
        let rows = db(statement.query_map([session], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        }))?;
        let mut updates = Vec::new();
        for row in rows {
            let (id, value) = db(row)?;
            let mut metric: Metric = serde_json::from_str(&value).map_err(|_| "invalid_metric")?;
            if metric.judge_status == JudgeStatus::Pending
                || (metric.assessment.is_none() && !metric.assessment_failed)
            {
                metric.judge_status = JudgeStatus::Cancelled;
                metric.assessment_failed = metric.assessment.is_none();
                metric.contextual = None;
                updates.push((
                    id,
                    serde_json::to_string(&metric).map_err(|_| "invalid_metric")?,
                ));
            }
        }
        drop(statement);
        for (id, value) in updates {
            db(self.connection.execute(
                "UPDATE metrics SET metric=?1 WHERE id=?2",
                params![value, id],
            ))?;
        }
        Ok(())
    }
    pub fn clear(&mut self) -> ReviewResult<()> {
        db(self.connection.execute_batch("DELETE FROM metrics; DELETE FROM counters; UPDATE settings SET generation=generation+1 WHERE id=1; VACUUM;"))?;
        Ok(())
    }
    pub fn count(&mut self, kind: &str, now: i64) -> ReviewResult<()> {
        if ![
            "queue_full",
            "assessment_failed",
            "cancelled",
            "history_skipped",
        ]
        .contains(&kind)
        {
            return Err("invalid_counter".into());
        }
        db(self.connection.execute("INSERT INTO counters(day,kind,count) VALUES(?1,?2,1) ON CONFLICT(day,kind) DO UPDATE SET count=count+1",params![now.div_euclid(DAY_MS),kind]))?;
        Ok(())
    }
}
impl MetricsStore for SqliteMetrics {
    fn record(
        &mut self,
        id: &str,
        session: &str,
        metric: &Metric,
        generation: u64,
        now: i64,
    ) -> ReviewResult<bool> {
        if metric.timestamp_ms < now - 30 * DAY_MS || metric.timestamp_ms > now {
            return Err("invalid_metric_time".into());
        }
        for value in [
            id,
            session,
            metric.model_fingerprint.as_str(),
            metric.rubric_fingerprint.as_str(),
            metric.policy_fingerprint.as_str(),
            metric.marker_fingerprint.as_str(),
        ]
        .into_iter()
        .chain(metric.judge_fingerprint.as_deref())
        {
            if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("invalid_fingerprint".into());
            }
        }
        let transaction = db(self.connection.transaction())?;
        let (live, history, current): (bool, bool, i64) = db(transaction.query_row(
            "SELECT live,history,generation FROM settings WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ))?;
        if current as u64 != generation
            || !(match metric.source {
                ObservationSource::Live => live,
                ObservationSource::History => history,
            })
        {
            return Ok(false);
        }
        let sampled = metric.judge_status != JudgeStatus::NotSampled;
        let source = if metric.source == ObservationSource::Live {
            "live"
        } else {
            "history"
        };
        db(transaction.execute("INSERT INTO metrics(id,session,timestamp_ms,source,sampled,metric) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET metric=excluded.metric,sampled=excluded.sampled",params![id,session,metric.timestamp_ms,source,sampled,serde_json::to_string(metric).map_err(|_|"invalid_metric")?]))?;
        db(transaction.commit())?;
        Ok(true)
    }
    fn export(&mut self, now: i64) -> ReviewResult<serde_json::Value> {
        self.expire(now)?;
        let mut statement = db(self
            .connection
            .prepare("SELECT metric FROM metrics ORDER BY timestamp_ms"))?;
        let rows = db(statement.query_map([], |r| r.get::<_, String>(0)))?;
        let mut metrics = Vec::new();
        for row in rows {
            metrics.push(serde_json::from_str::<Metric>(&db(row)?).map_err(|_| "invalid_metric")?);
        }
        let mut counters = std::collections::BTreeMap::new();
        let mut statement = db(self
            .connection
            .prepare("SELECT kind,SUM(count) FROM counters GROUP BY kind"))?;
        for row in
            db(statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))))?
        {
            let (k, v) = db(row)?;
            counters.insert(k, v);
        }
        let mut summary = serde_json::Map::new();
        for source in [ObservationSource::Live, ObservationSource::History] {
            let rows: Vec<_> = metrics.iter().filter(|m| m.source == source).collect();
            let mut categories = std::collections::BTreeMap::<String, usize>::new();
            let mut confidence_bins = [0usize; 10];
            let mut days = std::collections::BTreeMap::<i64, usize>::new();
            for row in &rows {
                *days.entry(row.timestamp_ms.div_euclid(DAY_MS)).or_default() += 1;
                if let Some(a) = &row.assessment
                    && let Some(c) = a.detector_confidence
                {
                    confidence_bins[((c * 10.0) as usize).min(9)] += 1;
                }
                if let Some(c) = &row.contextual {
                    *categories
                        .entry(
                            serde_json::to_value(c.before)
                                .map_err(|_| "invalid_metric")?
                                .as_str()
                                .ok_or("invalid_metric")?
                                .to_owned(),
                        )
                        .or_default() += 1;
                }
            }
            summary.insert(if source==ObservationSource::Live {"live"}else{"history"}.into(),serde_json::json!({"observations":rows.len(),"assessed":rows.iter().filter(|r|r.assessment.is_some()).count(),"replacement_count":rows.iter().filter_map(|r|r.assessment.as_ref()).map(|a|a.replacement_count).sum::<usize>(),"removed_characters":rows.iter().filter_map(|r|r.assessment.as_ref()).map(|a|a.removed_characters).sum::<usize>(),"judge_complete":rows.iter().filter(|r|r.judge_status==JudgeStatus::Complete).count(),"judge_unresolved":rows.iter().filter(|r|r.judge_status!=JudgeStatus::Complete && r.judge_status!=JudgeStatus::NotSampled).count(),"assessment_failed":rows.iter().filter(|r|r.assessment_failed).count(),"before_categories":categories,"detector_confidence_bins":confidence_bins,"daily_observations":days}));
        }
        Ok(
            serde_json::json!({"version":2,"policy_id":POLICY_ID,"retention_days":30,"consent":self.consent()?,"summary":summary,"metrics":metrics,"counters":counters,"policy_alignment_status":"inconclusive"}),
        )
    }
}
