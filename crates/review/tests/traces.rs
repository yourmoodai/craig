use craig_review::{
    now_ms,
    traces::{Decision, RETENTION_MS, Trace, TraceStore, random_id},
};
use serde_json::json;

fn event() -> Trace {
    Trace {
        schema: 1,
        offset_unit: "utf8".into(),
        id: random_id(),
        timestamp_ms: now_ms(),
        release: env!("CARGO_PKG_VERSION").into(),
        release_manifest_sha256: None,
        model_fingerprint: Some("a".repeat(64)),
        policy: "offensive_spans".into(),
        detector: "bert".into(),
        elapsed_ms: 1.0,
        outcome: "rewritten".into(),
        disposition: "processed".into(),
        code: None,
        decision: Some(Decision {
            detected: vec![craig::Span {
                start: 5,
                end: 10,
                score: 0.9,
            }],
            applied: vec![craig::Span {
                start: 5,
                end: 10,
                score: 0.9,
            }],
            threshold: 0.5,
            strategy: "fixed_phrase".into(),
            marker: Some("ya ya ya".into()),
        }),
        original: Some("🌱 idiot, keep this".into()),
        output: Some("🌱 ya ya ya, keep this".into()),
    }
}
#[test]
fn default_is_metadata_only_and_revocation_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().canonicalize().unwrap();
    let mut store = TraceStore::open(&path).unwrap();
    let e = event();
    let id = e.id.clone();
    store.record(e, 0).unwrap();
    let saved = store.get(&id).unwrap();
    assert!(saved["original"].is_null());
    assert!(saved["output"].is_null());
    assert!(saved["decision"]["marker"].is_null());
    assert!(
        !std::fs::read(path.join("traces.sqlite3"))
            .unwrap()
            .windows(b"keep this".len())
            .any(|w| w == b"keep this")
    );
    store.set_consent(true).unwrap();
    let generation = store.consent().unwrap().1;
    let e = event();
    let id = e.id.clone();
    store.record(e, generation).unwrap();
    assert_eq!(store.get(&id).unwrap()["original"], "🌱 idiot, keep this");
    store.set_consent(false).unwrap();
    let e = event();
    let id = e.id.clone();
    store.record(e, generation).unwrap();
    assert!(store.get(&id).unwrap()["original"].is_null());
}
#[test]
fn exact_decisions_retention_annotations_and_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().canonicalize().unwrap();
    let mut store = TraceStore::open(&path).unwrap();
    store.set_consent(true).unwrap();
    let generation = store.consent().unwrap().1;
    let mut invalid = event();
    invalid.output = Some("lost instructions".into());
    assert!(store.record(invalid, generation).is_err());
    let e = event();
    let id = e.id.clone();
    store.record(e, generation).unwrap();
    store.annotate(&id,json!({"profanity":"pass","targeted_abuse":"unresolved","preservation":"pass","note":"synthetic review"})).unwrap();
    assert!(
        store
            .annotate(&id, json!({"profanity":"approved"}))
            .is_err()
    );
    store.disposition(&id, "forwarded").unwrap();
    assert_eq!(store.get(&id).unwrap()["disposition"], "forwarded");
    store.prune(now_ms() + RETENTION_MS + 100).unwrap();
    assert!(store.get(&id).is_err());
    assert!(store.annotation(&id).unwrap().is_null());
    store.delete().unwrap();
    assert!(!store.consent().unwrap().0);
}
#[test]
fn competing_store_is_bounded_and_symlinks_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().canonicalize().unwrap();
    let store = TraceStore::open(&path).unwrap();
    assert!(TraceStore::open(&path).is_err());
    drop(store);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let target = path.join("traces.sqlite3");
        std::fs::rename(&target, path.join("old.sqlite3")).unwrap();
        symlink(path.join("old.sqlite3"), target).unwrap();
        assert!(TraceStore::open(&path).is_err());
    }
}
#[cfg(feature = "inspector")]
#[test]
fn parquet_roundtrip_and_annotation_join() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().canonicalize().unwrap();
    let mut store = TraceStore::open(&path).unwrap();
    store.set_consent(true).unwrap();
    let generation = store.consent().unwrap().1;
    let e = event();
    let id = e.id.clone();
    store.record(e, generation).unwrap();
    let rows = craig_review::inspector::parquet_events(&store, i64::MAX, "").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], id);
    assert_eq!(rows[0]["output"], "🌱 ya ya ya, keep this");
    store.annotate(&id,json!({"profanity":"pass","targeted_abuse":"unresolved","preservation":"pass","note":"synthetic"})).unwrap();
    let updated = craig_review::inspector::parquet_events(&store, i64::MAX, "").unwrap();
    assert_eq!(updated[0]["annotation"]["note"], "synthetic");
    assert_eq!(
        &std::fs::read(path.join("traces.parquet")).unwrap()[..4],
        b"PAR1"
    );
    assert!(
        craig_review::inspector::parquet_events(&store, i64::MAX, "failed")
            .unwrap()
            .is_empty()
    );
    store.delete().unwrap();
    assert!(!path.join("traces.parquet").exists());
}

#[test]
fn omission_and_custom_markers_preserve_exact_edits() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().canonicalize().unwrap();
    let mut store = TraceStore::open(&path).unwrap();
    store.set_consent(true).unwrap();
    let generation = store.consent().unwrap().1;
    for (strategy, marker, output) in [
        ("omit", None, "🌱 , keep this"),
        ("fixed_phrase", Some("🌼"), "🌱 🌼, keep this"),
    ] {
        let mut value = event();
        let decision = value.decision.as_mut().unwrap();
        decision.strategy = strategy.into();
        decision.marker = marker.map(str::to_owned);
        value.output = Some(output.into());
        let id = value.id.clone();
        store.record(value, generation).unwrap();
        assert_eq!(store.get(&id).unwrap()["output"], output);
    }
}

#[test]
fn interrupted_snapshot_is_recoverable_without_losing_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().canonicalize().unwrap();
    let mut store = TraceStore::open(&path).unwrap();
    let value = event();
    let id = value.id.clone();
    store.record(value, 0).unwrap();
    drop(store);
    std::fs::write(
        path.join("traces.parquet.partial"),
        b"incomplete synthetic projection",
    )
    .unwrap();
    let store = TraceStore::open(&path).unwrap();
    assert!(store.get(&id).is_ok());
    assert!(!path.join("traces.parquet.partial").exists());
}
