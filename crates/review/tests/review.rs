use craig_review::{
    DAY_MS,
    assessment::{Alignment, LanguageAssessment},
    history::{ClaudeHistory, HistorySource, window},
    policy::{Scenario, evidence},
    store::{JudgeStatus, Metric, MetricsStore, ObservationSource, SqliteMetrics, fingerprint},
};
use serde_json::json;

fn metric(timestamp: i64) -> Metric {
    Metric {
        source: ObservationSource::Live,
        timestamp_ms: timestamp,
        assessment: Some(LanguageAssessment {
            input_characters: 30,
            removed_characters: 5,
            replacement_count: 1,
            removed_proportion: 5.0 / 30.0,
            detector_confidence: Some(0.9),
            residual_confidence: Some(0.1),
            protected_abstention: false,
        }),
        judge_status: JudgeStatus::NotSampled,
        contextual: None,
        model_fingerprint: fingerprint(b"model"),
        rubric_fingerprint: fingerprint(b"rubric"),
        judge_fingerprint: None,
        policy_fingerprint: fingerprint(b"policy"),
        marker_fingerprint: fingerprint(b"marker"),
        assessment_failed: false,
        authorship_verified: true,
    }
}

#[test]
fn manually_moved_metrics_retain_data_without_consent() {
    let profile = tempfile::tempdir().unwrap();
    let now = 100 * DAY_MS;
    let mut store = SqliteMetrics::open(profile.path()).unwrap();
    assert!(!store.consent().unwrap().live);
    assert!(!store.consent().unwrap().history);
    let consent = store.set_consent(true, true).unwrap();
    let id = store.opaque("id", "migration-fixture");
    let session = store.opaque("session", "migration-session");
    assert!(
        store
            .record(&id, &session, &metric(now), consent.generation, now)
            .unwrap()
    );
    store.set_consent(false, false).unwrap();
    drop(store);

    // The old and new applications use the same database schema.
    let current = profile.path().join("craig");
    let retired = profile.path().join("middle-finger");
    std::fs::rename(&current, &retired).unwrap();
    assert!(!current.exists());
    std::fs::rename(&retired, &current).unwrap();
    let mut store = SqliteMetrics::open(profile.path()).unwrap();
    assert!(!store.consent().unwrap().live);
    assert!(!store.consent().unwrap().history);
    assert_eq!(
        store.export(now).unwrap()["summary"]["live"]["observations"],
        1
    );
    assert_eq!(
        store.export(now + 31 * DAY_MS).unwrap()["metrics"],
        json!([])
    );
    store.clear().unwrap();
    assert_eq!(store.export(now).unwrap()["metrics"], json!([]));
}

#[test]
fn consent_generation_prevents_late_writes_and_profiles_are_isolated() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut store = SqliteMetrics::open(a.path()).unwrap();
    let mut other = SqliteMetrics::open(b.path()).unwrap();
    let now = 100 * DAY_MS;
    let row = metric(now);
    let id = store.opaque("id", "synthetic-original");
    let session = store.opaque("session", "synthetic-session");
    assert!(!store.record(&id, &session, &row, 0, now).unwrap());
    let consent = store.set_consent(true, false).unwrap();
    assert!(
        store
            .record(&id, &session, &row, consent.generation, now)
            .unwrap()
    );
    assert!(
        store
            .record(&id, &session, &row, consent.generation, now)
            .unwrap()
    );
    assert_eq!(
        store.export(now).unwrap()["summary"]["live"]["observations"],
        1
    );
    assert_eq!(
        other.export(now).unwrap()["metrics"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_ne!(id, other.opaque("id", "synthetic-original"));
    store.set_consent(false, false).unwrap();
    assert!(
        !store
            .record(&id, &session, &row, consent.generation, now)
            .unwrap()
    );
    store.clear().unwrap();
    assert_eq!(store.export(now).unwrap()["metrics"], json!([]));
    assert!(
        !store
            .record(&id, &session, &row, consent.generation, now)
            .unwrap()
    );
    let bytes = std::fs::read(a.path().join("craig/metrics.sqlite3")).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-original"));
}
#[test]
fn retention_and_numeric_only_storage() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SqliteMetrics::open(dir.path()).unwrap();
    let now = 100 * DAY_MS;
    let consent = store.set_consent(true, true).unwrap();
    let id = store.opaque("id", "a");
    let session = store.opaque("session", "b");
    assert!(
        store
            .record(
                "raw text is forbidden",
                &session,
                &metric(now),
                consent.generation,
                now
            )
            .is_err()
    );
    assert!(
        store
            .record(&id, &session, &metric(now + 1), consent.generation, now)
            .is_err()
    );
    store
        .record(
            &id,
            &session,
            &metric(now - 30 * DAY_MS),
            consent.generation,
            now,
        )
        .unwrap();
    assert_eq!(
        store.export(now).unwrap()["metrics"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        store.export(now + 1).unwrap()["metrics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path().join("craig/metrics.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[test]
fn dates_are_prompt_dates_not_file_activity() {
    let profile = tempfile::tempdir().unwrap();
    let now = 100 * DAY_MS;
    for project in ["first", "second"] {
        std::fs::create_dir_all(profile.path().join("projects").join(project)).unwrap();
    }
    let make = |id: &str, timestamp: i64, version: &str| {
        json!({"type":"user","uuid":id,"sessionId":"session","version":version,"timestamp":chrono::DateTime::from_timestamp_millis(timestamp).unwrap().to_rfc3339(),"message":{"role":"user","content":"synthetic private prompt"}}).to_string()+"\n"
    };
    let input = make("edge", now - 3 * DAY_MS, "2.1.290")
        + &make("old", now - 3 * DAY_MS - 1, "2.1.290")
        + &make("future", now + 1, "2.1.290")
        + &make("unknown", now, "9.0.0")
        + &make("edge", now - 3 * DAY_MS, "2.1.290")
        + "{}\n{";
    std::fs::write(profile.path().join("projects/first/session.jsonl"), input).unwrap();
    let index = ClaudeHistory {
        profile: profile.path(),
    }
    .index(now, 3)
    .unwrap();
    assert_eq!(index.entries.len(), 1);
    assert_eq!(index.entries[0].uuid, "edge");
    assert_eq!(index.coverage.outside_window, 2);
    assert_eq!(index.coverage.unsupported, 1);
    assert_eq!(index.coverage.incomplete_tail, 1);
    assert!(
        !serde_json::to_string(&index)
            .unwrap()
            .contains("synthetic private")
    );
    assert!(window(now, 6, now).is_err());
    assert!(window(now, 0, now).is_err());
    assert!(window(now, 5, now - 5 * DAY_MS).unwrap());
}
#[test]
fn history_skips_symlinked_projects() {
    #[cfg(unix)]
    {
        let profile = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(profile.path().join("projects")).unwrap();
        std::os::unix::fs::symlink(outside.path(), profile.path().join("projects/work")).unwrap();
        let index = ClaudeHistory {
            profile: profile.path(),
        }
        .index(100 * DAY_MS, 3)
        .unwrap();
        assert_eq!(index.coverage.excluded, 1);
        assert!(index.entries.is_empty());
    }
}
#[test]
fn provisional_or_uncalibrated_policy_checks_remain_inconclusive() {
    let mut rows = vec![Scenario {
        id: "frustration".into(),
        category: "frustration".into(),
        prompts: vec!["Please fix this broken test.".into()],
        expected_replacements: vec![Some("Please fix this broken test.".into())],
        expected: Alignment::NoConcernObserved,
        reviewed: false,
    }];
    assert_eq!(
        evidence(&rows, &[Alignment::NoConcernObserved], &[true], true).unwrap()["policy_alignment_status"],
        "inconclusive"
    );
    rows[0].reviewed = true;
    assert_eq!(
        evidence(&rows, &[Alignment::NoConcernObserved], &[true], false).unwrap()["policy_alignment_status"],
        "inconclusive"
    );
    let result = evidence(&rows, &[Alignment::NoConcernObserved], &[true], true).unwrap();
    assert_eq!(result["policy_alignment_status"], "pass");
    assert_eq!(result["version"], 2);
    assert_eq!(
        evidence(&rows, &[Alignment::NoConcernObserved], &[false], true).unwrap()["replacement_correctness_status"],
        "fail"
    );
}

#[cfg(feature = "judge")]
#[test]
fn worker_queue_is_bounded_and_cancel_clears_pending_text() {
    use craig_review::worker::{Observation, Request, Worker};
    let dir = tempfile::tempdir().unwrap();
    let worker = Worker::new(dir.path()).unwrap();
    let result = worker
        .handle(Request::Consent {
            live: false,
            history: true,
        })
        .unwrap();
    let generation = result["consent"]["generation"].as_u64().unwrap();
    for i in 0..5 {
        let observation = Observation {
            id: format!("row-{i}"),
            session: "session".into(),
            generation,
            timestamp_ms: craig_review::now_ms(),
            source: ObservationSource::History,
            original: "synthetic private prompt".into(),
            replacement: None,
            assessment: None,
            context: vec![],
            authorship_verified: false,
        };
        let result = worker.handle(Request::Observe { observation }).unwrap();
        assert_eq!(result["status"], if i == 4 { "busy" } else { "accepted" });
    }
    worker
        .handle(Request::Consent {
            live: false,
            history: false,
        })
        .unwrap();
    assert!(!worker.busy());
    let report = worker.handle(Request::Export).unwrap();
    assert!(!report.to_string().contains("synthetic private prompt"));
    assert!(
        report["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["judge_status"] == "cancelled")
    );
}
