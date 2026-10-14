use std::process::Command;

fn lab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_craig-lab"))
}

#[test]
fn corpus_command_refuses_to_overwrite_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    assert!(
        lab()
            .arg("bootstrap")
            .arg("--output")
            .arg(&corpus)
            .output()
            .unwrap()
            .status
            .success()
    );
    let original = std::fs::read(&corpus).unwrap();
    assert!(
        !lab()
            .arg("bootstrap")
            .arg("--output")
            .arg(&corpus)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(std::fs::read(corpus).unwrap(), original);
}

#[test]
fn failed_setup_never_creates_a_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    assert!(
        lab()
            .arg("bootstrap")
            .arg("--output")
            .arg(&corpus)
            .output()
            .unwrap()
            .status
            .success()
    );
    let output = dir.path().join("candidate");
    let result = lab()
        .args([
            "train",
            "--model",
            "/missing-synthetic-model",
            "--policy",
            "directed_abuse",
            "--epochs",
            "1",
            "--corpus",
        ])
        .arg(corpus)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output.exists());
    assert_eq!(
        String::from_utf8(result.stderr).unwrap(),
        "craig-lab: base_model_unavailable\n"
    );
}

#[test]
#[ignore = "requires explicitly downloaded local model artifacts"]
fn real_model_training_export_and_policy_selection() {
    let base = std::env::var("CRAIG_MODEL_DIR").expect("set CRAIG_MODEL_DIR");
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    assert!(
        lab()
            .arg("bootstrap")
            .arg("--output")
            .arg(&corpus)
            .output()
            .unwrap()
            .status
            .success()
    );
    // Keep scenario families intact in the reduced integration corpus.
    let data = std::fs::read_to_string(&corpus).unwrap();
    let rows: Vec<_> = data
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|r| {
            ["ordinary", "directed", "profanity"].contains(&r["category"].as_str().unwrap())
        })
        .collect();
    std::fs::write(
        &corpus,
        rows.iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    for policy in ["offensive_spans", "directed_abuse"] {
        let output = dir.path().join(policy);
        let result = lab()
            .args([
                "train", "--model", &base, "--policy", policy, "--epochs", "1", "--corpus",
            ])
            .arg(&corpus)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["encoder_unchanged"], true);
        assert_eq!(report["classifier_changed"], true);
        assert!(report["export_probability_max_error"].as_f64().unwrap() <= 0.0001);
        let matching = policy.parse().unwrap();
        assert!(craig::ModelDetector::load_policy(&output, matching).is_ok());
        let different = if policy == "directed_abuse" {
            craig::Policy::OffensiveSpans
        } else {
            craig::Policy::DirectedAbuse
        };
        assert!(matches!(
            craig::ModelDetector::load_policy(&output, different),
            Err(craig::Error::PolicyUnavailable)
        ));
        std::fs::write(output.join("head.safetensors"), b"tampered").unwrap();
        assert!(matches!(
            craig::ModelDetector::load_policy(&output, matching),
            Err(craig::Error::ModelIncompatible)
        ));
    }
}
