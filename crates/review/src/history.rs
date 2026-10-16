use crate::{DAY_MS, ReviewResult};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryCapabilities {
    pub agent: String,
    pub all_local_sessions: bool,
    pub prompt_timestamps: bool,
    pub original_before_replacement: bool,
    pub supported_versions: Vec<String>,
}
pub fn capabilities() -> HistoryCapabilities {
    HistoryCapabilities {
        agent: "claude".into(),
        all_local_sessions: true,
        prompt_timestamps: true,
        original_before_replacement: false,
        supported_versions: vec!["2.1.290".into()],
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub session_id: String,
    pub uuid: String,
    pub timestamp_ms: i64,
    pub authorship_verified: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Coverage {
    pub files: usize,
    pub malformed: usize,
    pub unsupported: usize,
    pub missing_timestamp: usize,
    pub outside_window: usize,
    pub inaccessible: usize,
    pub excluded: usize,
    pub incomplete_tail: usize,
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryIndex {
    pub capabilities: HistoryCapabilities,
    pub now_ms: i64,
    pub days: u8,
    pub entries: Vec<HistoryEntry>,
    pub coverage: Coverage,
}
pub trait HistorySource {
    fn index(&self, now: i64, days: u8) -> ReviewResult<HistoryIndex>;
}
pub struct ClaudeHistory<'a> {
    pub profile: &'a Path,
}
pub fn window(now: i64, days: u8, timestamp: i64) -> ReviewResult<bool> {
    if !(1..=5).contains(&days) || now < 0 {
        return Err("invalid_history_window".into());
    }
    let start = (now - i64::from(days) * DAY_MS).max(0);
    Ok(timestamp >= start && timestamp <= now)
}
impl HistorySource for ClaudeHistory<'_> {
    fn index(&self, now: i64, days: u8) -> ReviewResult<HistoryIndex> {
        window(now, days, now)?;
        let mut index = HistoryIndex {
            capabilities: capabilities(),
            now_ms: now,
            days,
            entries: Vec::new(),
            coverage: Coverage::default(),
        };
        let projects = self.profile.join("projects");
        if !projects.exists() {
            index.coverage.inaccessible += 1;
            return Ok(index);
        }
        if projects.is_symlink() {
            return Err("unsafe_history_path".into());
        }
        let directories = std::fs::read_dir(&projects).map_err(|_| "history_unavailable")?;
        let mut unique = BTreeMap::new();
        let started = std::time::Instant::now();
        'scan: for dir in directories {
            if started.elapsed().as_secs() >= 5 {
                index.coverage.truncated = true;
                break;
            }
            let Ok(dir) = dir else {
                index.coverage.inaccessible += 1;
                continue;
            };
            let Ok(kind) = dir.file_type() else {
                index.coverage.inaccessible += 1;
                continue;
            };
            if !kind.is_dir() || kind.is_symlink() {
                index.coverage.excluded += 1;
                continue;
            }
            let Ok(files) = std::fs::read_dir(dir.path()) else {
                index.coverage.inaccessible += 1;
                continue;
            };
            for file in files {
                if started.elapsed().as_secs() >= 5 {
                    index.coverage.truncated = true;
                    break 'scan;
                }
                let Ok(file) = file else {
                    index.coverage.inaccessible += 1;
                    continue;
                };
                let Ok(kind) = file.file_type() else {
                    index.coverage.inaccessible += 1;
                    continue;
                };
                if !kind.is_file()
                    || kind.is_symlink()
                    || file.path().extension().is_none_or(|s| s != "jsonl")
                {
                    continue;
                }
                let Ok(input) = std::fs::File::open(file.path()) else {
                    index.coverage.inaccessible += 1;
                    continue;
                };
                index.coverage.files += 1;
                let mut reader = BufReader::new(input);
                loop {
                    if unique.len() >= 10_000 || started.elapsed().as_secs() >= 5 {
                        index.coverage.truncated = true;
                        break 'scan;
                    }
                    // A missing newline must not cause transcript-sized allocation.
                    let mut line = Vec::new();
                    use std::io::Read;
                    let read = (&mut reader).take(524_289).read_until(b'\n', &mut line); // 512 KiB plus overflow sentinel
                    match read {
                        Ok(0) => break,
                        Err(_) => {
                            index.coverage.inaccessible += 1;
                            break;
                        }
                        _ => {}
                    }
                    if line.len() > 524_288 {
                        index.coverage.malformed += 1;
                        break;
                    }
                    if line.last() != Some(&b'\n') {
                        index.coverage.incomplete_tail += 1;
                        break;
                    }
                    let Ok(row) = serde_json::from_slice::<serde_json::Value>(&line) else {
                        index.coverage.malformed += 1;
                        continue;
                    };
                    if row["type"] != "user"
                        || row["isMeta"] == true
                        || row["isSidechain"] == true
                        || row["isCompactSummary"] == true
                    {
                        index.coverage.excluded += 1;
                        continue;
                    }
                    if row["version"] != "2.1.290" {
                        index.coverage.unsupported += 1;
                        continue;
                    }
                    let Some(timestamp) = row["timestamp"]
                        .as_str()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|d| d.timestamp_millis())
                    else {
                        index.coverage.missing_timestamp += 1;
                        continue;
                    };
                    if !window(now, days, timestamp)? {
                        index.coverage.outside_window += 1;
                        continue;
                    }
                    let (Some(session), Some(uuid)) =
                        (row["sessionId"].as_str(), row["uuid"].as_str())
                    else {
                        index.coverage.malformed += 1;
                        continue;
                    };
                    if !valid_id(session) || !valid_id(uuid) {
                        index.coverage.malformed += 1;
                        continue;
                    }
                    if file.path().file_stem().and_then(|s| s.to_str()) != Some(session) {
                        index.coverage.malformed += 1;
                        continue;
                    }
                    let entry = HistoryEntry {
                        session_id: session.into(),
                        uuid: uuid.into(),
                        timestamp_ms: timestamp,
                        authorship_verified: false,
                    };
                    unique.insert((session.to_string(), uuid.to_string()), entry);
                }
            }
        }
        index.entries = unique.into_values().collect();
        index.entries.sort_by_key(|e| e.timestamp_ms);
        Ok(index)
    }
}
pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
