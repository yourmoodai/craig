use crate::{
    ReviewResult,
    assessment::ContextualAssessment,
    policy::{self, JudgeRun, PreparedPolicy},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

#[derive(Default, serde::Serialize)]
struct Agreement {
    agree: usize,
    disagree: usize,
    unresolved: usize,
    agreement_rate: Option<f64>,
}
pub fn dimensions(
    left: Option<&ContextualAssessment>,
    right: Option<&ContextualAssessment>,
) -> BTreeMap<String, &'static str> {
    let left = serde_json::to_value(left).expect("categorical assessment");
    let right = serde_json::to_value(right).expect("categorical assessment");
    ["before", "after", "alignment", "preservation"]
        .into_iter()
        .map(|key| {
            let pair = (left[key].as_str(), right[key].as_str());
            let state = match pair {
                (Some(a), Some(b))
                    if !["uncertain", "inconclusive"].contains(&a)
                        && !["uncertain", "inconclusive"].contains(&b) =>
                {
                    if a == b {
                        "agree"
                    } else {
                        "disagree"
                    }
                }
                _ => "unresolved",
            };
            (key.to_owned(), state)
        })
        .collect()
}
pub fn report(prepared: &PreparedPolicy, runs: &[JudgeRun; 2]) -> ReviewResult<Value> {
    if runs
        .iter()
        .any(|r| r.judgments.len() != prepared.rows.len() || r.errors.len() != prepared.rows.len())
    {
        return Err("incompatible_judge_runs".into());
    }
    let mut summary: BTreeMap<String, Agreement> = BTreeMap::new();
    let mut outcomes = Vec::new();
    let mut review_queue = Vec::new();
    for (i, row) in prepared.rows.iter().enumerate() {
        if prepared.rows.len() != prepared.exact.len() || prepared.rows.is_empty() {
            return Err("incompatible_judge_runs".into());
        }
        if runs.iter().any(|r| {
            r.judgments[i].len() != row.prompts.len() || r.errors[i].len() != row.prompts.len()
        }) {
            return Err("incompatible_judge_runs".into());
        }
        let mut turns = Vec::new();
        for turn in 0..row.prompts.len() {
            let states = dimensions(
                runs[0].judgments[i][turn].as_ref(),
                runs[1].judgments[i][turn].as_ref(),
            );
            for (dimension, state) in &states {
                let counts = summary.entry(dimension.clone()).or_default();
                match *state {
                    "agree" => counts.agree += 1,
                    "disagree" => counts.disagree += 1,
                    _ => counts.unresolved += 1,
                }
                if *state != "agree" {
                    review_queue.push(
                        json!({"id":row.id,"turn":turn,"dimension":dimension,"reason":state}),
                    );
                }
            }
            turns.push(json!({"turn":turn,"judgments":[runs[0].judgments[i][turn],runs[1].judgments[i][turn]],"errors":[runs[0].errors[i][turn],runs[1].errors[i][turn]],"cross_model_agreement":states}));
        }
        outcomes.push(json!({"id":row.id,"category":row.category,"exact_replacement":prepared.exact[i],"turns":turns}));
        if !prepared.exact[i] {
            review_queue.push(json!({"id":row.id,"reason":"reference_replacement_mismatch"}));
        }
    }
    for counts in summary.values_mut() {
        let resolved = counts.agree + counts.disagree;
        counts.agreement_rate = (resolved > 0).then(|| counts.agree as f64 / resolved as f64);
    }
    let references: Vec<_> = runs
        .iter()
        .map(|run| {
            let actual: Vec<_> = run
                .judgments
                .iter()
                .map(|s| policy::sequence_alignment(s))
                .collect();
            policy::evidence(&prepared.rows, &actual, &prepared.exact, false)
        })
        .collect::<ReviewResult<_>>()?;
    let mut result = prepared.provenance.clone();
    result.as_object_mut().unwrap().extend(json!({
        "version":2,"scope":"measured_policy_alignment","policy_id":policy::POLICY_ID,
        "policy_alignment_status":"inconclusive","judge_calibrated":false,"labels_reviewed":prepared.rows.iter().all(|r|r.reviewed),
        "enforcement_guaranteed":false,"replacement_correctness_status":if prepared.exact.contains(&false){"fail"}else{"pass"},
        "reference_replacement_mismatches":prepared.exact.iter().filter(|v|!**v).count(),
        "judges":[runs[0].provenance,runs[1].provenance],"reference_label_comparisons":references,
        "cross_model_agreement":summary,"outcomes":outcomes,"review_queue":review_queue,
        "judge_failed":runs.iter().any(JudgeRun::failed)
    }).as_object().unwrap().clone());
    Ok(result)
}
pub fn evaluate(corpus: &Path, left: &Path, right: &Path, output: &Path) -> ReviewResult<()> {
    if output.exists() {
        return Err("output_exists_or_unavailable".into());
    }
    let identity = |path: &Path| {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<crate::judge::JudgeConfig>(&b).ok())
            .map(|c| c.model_id)
    };
    if let (Some(a), Some(b)) = (identity(left), identity(right))
        && a == b
    {
        return Err("independent_judges_required".into());
    }
    let prepared = policy::prepare(corpus)?;
    // Judges share identical inputs but never reside in memory together.
    let runs = [
        policy::run_judge(&prepared, Some(left)),
        policy::run_judge(&prepared, Some(right)),
    ];
    policy::write_report(output, &report(&prepared, &runs)?)?;
    if runs.iter().any(JudgeRun::failed) {
        return Err("policy_judge_unavailable".into());
    }
    if prepared.exact.contains(&false) {
        return Err("policy_fixture_failed".into());
    }
    Ok(())
}
