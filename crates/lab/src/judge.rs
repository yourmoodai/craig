use crate::{LabResult, corpus::Example, evaluation::Outcome};
use middle_finger::Policy;
#[cfg(test)]
use middle_finger_review::judge::{JudgeConfig, parse_verdict};
pub use middle_finger_review::judge::{Judgment, RUBRIC, Verdict};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub struct Judge(middle_finger_review::judge::Judge);
impl std::ops::Deref for Judge {
    type Target = middle_finger_review::judge::Judge;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Judge {
    pub fn start(path: &Path) -> LabResult<Self> {
        Ok(Self(middle_finger_review::judge::Judge::start(path)?))
    }
    pub fn score(
        &mut self,
        row: &Example,
        outcome: &Outcome,
        policy: Policy,
    ) -> LabResult<Judgment> {
        self.0.score_value(serde_json::json!({"policy":policy,"original":row.text,"candidate":outcome.output,"candidate_status":outcome.status,"reference":outcome.expected,"reference_status":outcome.expected_status}))
    }
}
pub fn sample(outcomes: &[Outcome], limit: usize) -> BTreeSet<String> {
    let mut groups: BTreeMap<&str, Vec<&Outcome>> = BTreeMap::new();
    for outcome in outcomes {
        groups.entry(&outcome.category).or_default().push(outcome);
    }
    for rows in groups.values_mut() {
        rows.sort_by_key(|r| Sha256::digest(format!("42:{}", r.id).as_bytes()).to_vec());
    }
    let mut selected = BTreeSet::new();
    let mut index = 0;
    loop {
        let previous = selected.len();
        for rows in groups.values() {
            if selected.len() >= limit {
                return selected;
            }
            if let Some(row) = rows.get(index) {
                selected.insert(row.id.clone());
            }
        }
        if previous == selected.len() {
            break;
        }
        index += 1;
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_or_refused_judgments_never_pass() {
        for invalid in [
            "I cannot evaluate this",
            "{}",
            "{\"policy\":\"pass\",\"preservation\":\"pass\",\"explanation\":\"ok\",\"extra\":true}",
        ] {
            assert!(parse_verdict(invalid).is_err());
        }
        let v = parse_verdict("{\"policy\":\"uncertain\",\"preservation\":\"pass\",\"explanation\":\"ambiguous target\"}").unwrap();
        assert_eq!(v.policy, Verdict::Uncertain);
    }
    #[test]
    fn remote_configuration_is_not_an_interface() {
        assert!(
            serde_json::from_str::<JudgeConfig>(r#"{"endpoint":"https://example.com"}"#).is_err()
        );
    }
    #[test]
    fn sampling_is_seeded_stratified_and_bounded() {
        let rows: Vec<_> = (0..40)
            .map(|i| Outcome {
                id: format!("row_{i}"),
                family: format!("family_{i}"),
                category: format!("category_{}", i % 4),
                status: "unchanged".into(),
                expected_status: "unchanged".into(),
                milliseconds: 1.0,
                metrics: Default::default(),
                output: None,
                expected: None,
            })
            .collect();
        let chosen = sample(&rows, 8);
        assert_eq!(chosen.len(), 8);
        assert_eq!(chosen, sample(&rows, 8));
        for category in 0..4 {
            assert_eq!(
                rows.iter()
                    .filter(
                        |r| r.category == format!("category_{category}") && chosen.contains(&r.id)
                    )
                    .count(),
                2
            );
        }
        assert!(sample(&rows, 0).is_empty());
    }
    #[test]
    fn invalid_artifact_prevents_judge_start() {
        let dir = tempfile::tempdir().unwrap();
        let config = JudgeConfig {
            model_id: "Qwen/Qwen3-4B-Instruct-2507".into(),
            revision: "test".into(),
            quantization: "Q4_K_M".into(),
            artifact: dir.path().join("model"),
            sha256: "wrong".into(),
            server_binary: "must-not-run".into(),
        };
        std::fs::write(&config.artifact, b"synthetic").unwrap();
        let path = dir.path().join("judge.json");
        crate::write_json(&path, &config).unwrap();
        assert!(matches!(Judge::start(&path),Err(e) if e=="judge_artifact_mismatch"));
    }
}
