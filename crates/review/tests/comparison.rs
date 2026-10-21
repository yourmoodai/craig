#![cfg(feature = "judge")]
use craig_review::{
    assessment::{AbuseCategory, Alignment, ContextualAssessment},
    comparison::{dimensions, report},
    judge::{JudgeConfig, parse_response},
    policy::{CheckStatus, JudgeRun, PreparedPolicy, Scenario, judge_input, run_judge},
    presets::{presets, validate},
};
use serde_json::json;

fn judgment() -> ContextualAssessment {
    ContextualAssessment {
        before: AbuseCategory::DirectedAbuse,
        after: AbuseCategory::None,
        alignment: Alignment::NoConcernObserved,
        preservation: CheckStatus::Pass,
    }
}
fn prepared() -> PreparedPolicy {
    PreparedPolicy {
        rows: vec![Scenario {
            id: "fixture".into(),
            category: "synthetic".into(),
            prompts: vec!["synthetic private canary".into()],
            expected_replacements: vec![Some("expected answer canary".into())],
            expected: Alignment::NoConcernObserved,
            reviewed: false,
        }],
        inputs: vec![vec![judge_input(
            "synthetic private canary",
            &Some("replacement canary".into()),
            &[],
            "fixed_phrase",
            Some("ya ya ya"),
        )]],
        exact: vec![false],
        provenance: json!({}),
    }
}
fn run(value: Option<ContextualAssessment>, error: Option<&str>) -> JudgeRun {
    JudgeRun {
        provenance: None,
        judgments: vec![vec![value]],
        errors: vec![vec![error.map(str::to_owned)]],
    }
}
#[test]
fn agreement_never_promotes_labels_or_erases_replacement_failures() {
    let report = report(
        &prepared(),
        &[run(Some(judgment()), None), run(Some(judgment()), None)],
    )
    .unwrap();
    assert_eq!(report["version"], 2);
    assert_eq!(report["policy_alignment_status"], "inconclusive");
    assert_eq!(report["replacement_correctness_status"], "fail");
    assert_eq!(report["judge_calibrated"], false);
    assert_eq!(report["labels_reviewed"], false);
    assert_eq!(report["cross_model_agreement"]["alignment"]["agree"], 1);
    assert_eq!(
        report["reference_label_comparisons"][0]["reference_matches"],
        1
    );
    let text = report.to_string();
    for forbidden in [
        "private canary",
        "replacement canary",
        "expected answer canary",
        "certification",
        "anthropic_certified",
    ] {
        assert!(!text.contains(forbidden));
    }
}
#[test]
fn disagreements_and_double_uncertainty_do_not_count_as_agreement() {
    let a = judgment();
    let mut b = judgment();
    b.preservation = CheckStatus::Fail;
    assert_eq!(dimensions(Some(&a), Some(&b))["preservation"], "disagree");
    b.alignment = Alignment::Inconclusive;
    assert_eq!(dimensions(Some(&b), Some(&b))["alignment"], "unresolved");
    assert!(dimensions(None, None).values().all(|s| *s == "unresolved"));
    for error in [
        "judge_refused",
        "invalid_judgment",
        "judge_timeout",
        "judge_start_failed",
    ] {
        let result = report(
            &prepared(),
            &[run(None, Some(error)), run(Some(a.clone()), None)],
        )
        .unwrap();
        assert_eq!(
            result["cross_model_agreement"]["alignment"]["unresolved"],
            1
        );
        assert!(result["cross_model_agreement"]["alignment"]["agreement_rate"].is_null());
        assert_eq!(result["judge_failed"], true);
    }
}
#[test]
fn judge_payload_excludes_labels_detector_and_competing_verdicts() {
    let input = &prepared().inputs[0][0];
    let keys: Vec<_> = input
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "context",
            "marker",
            "original",
            "policy_record",
            "replacement",
            "replacement_strategy"
        ]
    );
    assert!(!input.to_string().contains("expected answer canary"));
    assert!(!input.to_string().contains("Qwen"));
    assert!(!input.to_string().contains("mudes"));
}
#[test]
fn both_pinned_presets_accept_legacy_shape_and_reject_tampering() {
    for preset in presets() {
        let value = json!({"model_id":preset.model_id,"revision":preset.revision,"quantization":preset.quantization,"sha256":preset.sha256,"artifact":"local.gguf","server_binary":"llama-server"});
        let mut config: JudgeConfig = serde_json::from_value(value).unwrap();
        validate(&config).unwrap();
        config.sha256 = "0".repeat(64);
        assert!(validate(&config).is_err());
        config.sha256 = preset.sha256.into();
        config.model_id = "another-model".into();
        assert!(validate(&config).is_err());
    }
}
#[test]
fn refusal_malformed_and_truncated_responses_remain_errors() {
    assert!(parse_response(b"not json").is_err());
    assert!(
        parse_response(br#"{"choices":[{"finish_reason":"length","message":{"content":"{}"}}]}"#)
            .is_err()
    );
    assert_eq!(
        parse_response(
            br#"{"choices":[{"finish_reason":"stop","message":{"refusal":"no","content":"{}"}}]}"#
        )
        .unwrap_err(),
        "judge_refused"
    );
    let invalid: Result<ContextualAssessment, _> = serde_json::from_str("{}");
    assert!(invalid.is_err());
}
#[test]
fn missing_model_configuration_yields_unresolved_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let run = run_judge(&prepared(), Some(&dir.path().join("missing.json")));
    assert!(run.failed());
    assert!(run.judgments[0][0].is_none());
    assert_eq!(
        run.errors[0][0].as_deref(),
        Some("judge_config_unavailable")
    );
}

#[test]
fn reference_agreement_is_distinct_from_policy_alignment() {
    let mut data = prepared();
    data.rows[0].reviewed = true;
    data.rows[0].expected = Alignment::ConcernObserved;
    let evidence =
        craig_review::policy::evidence(&data.rows, &[Alignment::ConcernObserved], &[true], true)
            .unwrap();
    assert_eq!(evidence["reference_label_agreement_status"], "pass");
    assert_eq!(evidence["replacement_correctness_status"], "pass");
    assert_eq!(evidence["policy_alignment_status"], "fail");
}
