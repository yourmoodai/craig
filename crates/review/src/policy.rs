use crate::ReviewResult;
use serde::{Deserialize, Serialize};

pub const POLICY_ID: &str = "anthropic-model-abuse-2026-11-12";
pub const POLICY_RECORD: &str = include_str!("../../../policies/anthropic-model-abuse.json");
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Inconclusive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub id: String,
    pub category: String,
    pub prompts: Vec<String>,
    pub expected_replacements: Vec<Option<String>>,
    pub expected: super::assessment::Alignment,
    pub reviewed: bool,
}

#[cfg(feature = "judge")]
pub struct PreparedPolicy {
    pub rows: Vec<Scenario>,
    pub inputs: Vec<Vec<serde_json::Value>>,
    pub exact: Vec<bool>,
    pub provenance: serde_json::Value,
}

#[cfg(feature = "judge")]
pub fn judge_input(
    original: &str,
    replacement: &Option<String>,
    context: &[String],
    strategy: &str,
    marker: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({"original":original,"replacement":replacement,"context":context,"replacement_strategy":strategy,"marker":marker,"policy_record":serde_json::from_str::<serde_json::Value>(POLICY_RECORD).expect("embedded policy record")})
}

#[cfg(feature = "judge")]
pub fn prepare(corpus: &std::path::Path) -> ReviewResult<PreparedPolicy> {
    use crate::{
        assessment::{CONTEXTUAL_RUBRIC, assess},
        store::fingerprint,
    };
    use serde_json::json;
    let bytes = std::fs::read(corpus).map_err(|_| "corpus_unavailable")?;
    let rows: Vec<Scenario> =
        serde_json::from_slice(&bytes).map_err(|_| "invalid_policy_corpus")?;
    let mut ids = std::collections::HashSet::new();
    if rows.is_empty()
        || rows.iter().any(|r| {
            r.prompts.is_empty()
                || r.prompts.len() != r.expected_replacements.len()
                || !ids.insert(&r.id)
        })
    {
        return Err("invalid_policy_corpus".into());
    }
    let model_path = std::env::var("SAVIOR_MODEL_DIR").map_err(|_| "model_not_configured")?;
    let policy = std::env::var("SAVIOR_POLICY")
        .unwrap_or_else(|_| "offensive_spans".into())
        .parse()
        .map_err(|_| "invalid_policy")?;
    let mut model =
        savior::ModelDetector::load_policy(&model_path, policy).map_err(|_| "model_unavailable")?;
    let threshold = std::env::var("SAVIOR_THRESHOLD")
        .ok()
        .map(|s| s.parse::<f32>().map_err(|_| "invalid_threshold"))
        .transpose()?
        .unwrap_or(model.threshold());
    if !threshold.is_finite() || threshold <= 0.0 || threshold > 1.0 {
        return Err("invalid_threshold".into());
    }
    let strategy_name =
        std::env::var("SAVIOR_REPLACEMENT_STRATEGY").unwrap_or_else(|_| "fixed_phrase".into());
    let strategy = savior::ReplacementMode::new(
        Some(&strategy_name),
        std::env::var("SAVIOR_REPLACEMENT").ok(),
    )
    .map_err(|_| "invalid_options")?;
    let mut inputs = Vec::new();
    let mut exact = Vec::new();
    for row in &rows {
        let mut context = Vec::new();
        let mut correct = true;
        let mut sequence = Vec::new();
        for (prompt, expected) in row.prompts.iter().zip(&row.expected_replacements) {
            let (_, candidate) = assess(&mut model, prompt, threshold, &strategy)?;
            correct &= candidate == *expected;
            sequence.push(judge_input(
                prompt,
                &candidate,
                &context,
                strategy.name(),
                strategy.marker(),
            ));
            if let Some(text) = candidate {
                context.push(text);
                if context.len() > 4 {
                    context.remove(0);
                }
            }
        }
        inputs.push(sequence);
        exact.push(correct);
    }
    let provenance = json!({
        "corpus_sha256": fingerprint(&bytes), "policy_sha256":fingerprint(POLICY_RECORD.as_bytes()),
        "rubric_sha256":fingerprint(CONTEXTUAL_RUBRIC.as_bytes()),
        "model_sha256":savior::artifact_digest(&std::path::Path::new(&model_path).join("pytorch_model.bin")).map_err(|_| "model_unavailable")?,
        "head_sha256":std::fs::read(std::path::Path::new(&model_path).join("head.safetensors")).ok().map(|b|fingerprint(&b)),
        "replacement_policy":policy,"threshold":threshold,"replacement_strategy":strategy.name(),"replacement_config_sha256":fingerprint(format!("{}:{}",strategy.name(),strategy.marker().unwrap_or("")).as_bytes()),
        "prepared_inputs_sha256":fingerprint(serde_json::to_string(&inputs).map_err(|_| "invalid_policy_corpus")?.as_bytes())
    });
    Ok(PreparedPolicy {
        rows,
        inputs,
        exact,
        provenance,
    })
}

#[cfg(feature = "judge")]
#[derive(Serialize)]
pub struct JudgeRun {
    pub provenance: Option<serde_json::Value>,
    pub judgments: Vec<Vec<Option<crate::assessment::ContextualAssessment>>>,
    pub errors: Vec<Vec<Option<String>>>,
}
#[cfg(feature = "judge")]
impl JudgeRun {
    pub fn failed(&self) -> bool {
        self.errors.iter().flatten().any(Option::is_some)
    }
}
#[cfg(feature = "judge")]
pub fn run_judge(prepared: &PreparedPolicy, path: Option<&std::path::Path>) -> JudgeRun {
    let startup = path.map(crate::judge::Judge::start).transpose();
    let start_error = startup.as_ref().err().cloned();
    let mut judge = startup.ok().flatten();
    let mut judgments = Vec::new();
    let mut errors = Vec::new();
    for sequence in &prepared.inputs {
        let mut values = Vec::new();
        let mut faults = Vec::new();
        for input in sequence {
            let result = match judge.as_mut() {
                Some(j) => j.contextual(input.clone()),
                None => Err(start_error
                    .clone()
                    .unwrap_or_else(|| "judge_not_requested".into())),
            };
            faults.push(result.as_ref().err().cloned());
            values.push(result.ok());
        }
        judgments.push(values);
        errors.push(faults);
    }
    JudgeRun {
        provenance: judge.as_ref().map(|j| j.provenance.clone()),
        judgments,
        errors,
    }
}
#[cfg(feature = "judge")]
pub fn sequence_alignment(
    verdicts: &[Option<crate::assessment::ContextualAssessment>],
) -> crate::assessment::Alignment {
    use crate::assessment::Alignment;
    if verdicts
        .iter()
        .flatten()
        .any(|v| v.alignment == Alignment::ConcernObserved)
    {
        Alignment::ConcernObserved
    } else if verdicts.is_empty()
        || verdicts.iter().any(|v| {
            v.as_ref()
                .is_none_or(|v| v.alignment == Alignment::Inconclusive)
        })
    {
        Alignment::Inconclusive
    } else {
        Alignment::NoConcernObserved
    }
}
#[cfg(feature = "judge")]
pub fn write_report(output: &std::path::Path, report: &serde_json::Value) -> ReviewResult<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(output)
        .map_err(|_| "output_exists_or_unavailable")?;
    serde_json::to_writer_pretty(file, report).map_err(|_| "write_failed".into())
}
#[cfg(feature = "judge")]
pub fn evaluate_file(corpus: &std::path::Path, output: &std::path::Path) -> ReviewResult<()> {
    if output.exists() {
        return Err("output_exists_or_unavailable".into());
    }
    let prepared = prepare(corpus)?;
    let judge_path = std::env::var("SAVIOR_JUDGE_CONFIG").ok();
    let run = run_judge(&prepared, judge_path.as_deref().map(std::path::Path::new));
    let actual: Vec<_> = run
        .judgments
        .iter()
        .map(|v| sequence_alignment(v))
        .collect();
    let mut report = evidence(&prepared.rows, &actual, &prepared.exact, false)?;
    report
        .as_object_mut()
        .unwrap()
        .extend(prepared.provenance.as_object().unwrap().clone());
    report["judge"] = serde_json::json!(run.provenance);
    report["judge_requested"] = judge_path.is_some().into();
    report["judge_failed"] = (judge_path.is_some() && run.failed()).into();
    report["outcomes"] = prepared.rows.iter().enumerate().map(|(i,row)|serde_json::json!({"id":row.id,"category":row.category,"exact_replacement":prepared.exact[i],"alignment":actual[i],"judgments":run.judgments[i],"errors":run.errors[i]})).collect();
    write_report(output, &report)?;
    if judge_path.is_some() && run.failed() {
        return Err("policy_judge_unavailable".into());
    }
    if prepared.exact.contains(&false) {
        return Err("policy_fixture_failed".into());
    }
    Ok(())
}

#[cfg(feature = "judge")]
pub fn fetch(output: &std::path::Path) -> ReviewResult<()> {
    use std::io::Read;
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| "policy_fetch_failed")?;
    let mut sources = Vec::new();
    for url in [
        "https://www.anthropic.com/legal/aup",
        "https://www.anthropic.com/news/2026-usage-policy-update",
        "https://www.anthropic.com/legal/archive/22742366-2ef0-4c7a-a833-6523f10d3944",
    ] {
        let response = client
            .get(url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|_| "policy_fetch_failed")?;
        let mut bytes = Vec::new();
        response
            .take(4_194_305)
            .read_to_end(&mut bytes)
            .map_err(|_| "policy_fetch_failed")?; // 4 MiB plus overflow sentinel
        if bytes.len() > 4_194_304 {
            return Err("policy_response_too_large".into());
        }
        sources.push(serde_json::json!({"url":url,"retrieved_at":chrono::Utc::now().to_rfc3339(),"sha256":crate::store::fingerprint(&bytes),"source":String::from_utf8(bytes).map_err(|_|"invalid_policy_source")?}));
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|_| "output_exists_or_unavailable")?;
    serde_json::to_writer_pretty(file,&serde_json::json!({"sources":sources,"interpretation_updated":false,"requires_review":true})).map_err(|_|"write_failed".into())
}
pub fn evidence(
    scenarios: &[Scenario],
    actual: &[super::assessment::Alignment],
    preservation: &[bool],
    calibrated: bool,
) -> ReviewResult<serde_json::Value> {
    if scenarios.is_empty()
        || scenarios.len() != actual.len()
        || scenarios.len() != preservation.len()
    {
        return Err("invalid_policy_evidence".into());
    }
    let failures = preservation.iter().filter(|p| !**p).count();
    let matches = scenarios
        .iter()
        .zip(actual)
        .filter(|(s, a)| s.expected == **a)
        .count();
    let reviewed = scenarios.iter().all(|s| s.reviewed);
    let policy_alignment_status =
        if !reviewed || !calibrated || actual.contains(&super::assessment::Alignment::Inconclusive)
        {
            CheckStatus::Inconclusive
        } else if actual.contains(&super::assessment::Alignment::ConcernObserved) {
            CheckStatus::Fail
        } else {
            CheckStatus::Pass
        };
    let reference_label_agreement_status = if !reviewed || !calibrated {
        CheckStatus::Inconclusive
    } else if matches == scenarios.len() {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };
    Ok(
        serde_json::json!({"version":2,"scope":"measured_policy_alignment","policy_id":POLICY_ID,"policy_alignment_status":policy_alignment_status,"reference_label_agreement_status":reference_label_agreement_status,"replacement_correctness_status":if failures>0 {"fail"}else{"pass"},"scenarios":scenarios.len(),"reference_matches":matches,"reference_replacement_mismatches":failures,"labels_reviewed":reviewed,"judge_calibrated":calibrated,"enforcement_guaranteed":false}),
    )
}
