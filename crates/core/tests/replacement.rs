use craig::*;
use proptest::prelude::*;
fn span(start: usize, end: usize) -> Span {
    Span {
        start,
        end,
        score: 0.9,
    }
}
#[test]
fn replaces_only_detected_prose() {
    let text = "You idiot, do not change `src/main.rs`; keep 42 retries.";
    let result = apply_replacements(text, &[span(4, 9)], 0.5, &FixedMarker::default()).unwrap();
    assert_eq!(
        result.text,
        "You ya ya ya, do not change `src/main.rs`; keep 42 retries."
    );
}
#[test]
fn merges_repeated_windows_and_adjacent_words() {
    let text = "you stupid idiot, fix it";
    let result = apply_replacements(
        text,
        &[span(4, 10), span(11, 16), span(5, 8)],
        0.5,
        &FixedMarker::default(),
    )
    .unwrap();
    assert_eq!(result.text, "you ya ya ya, fix it");
    assert_eq!(result.replacement_count, 1);
}
#[test]
fn protects_literals() {
    for text in [
        "`idiot`",
        "\"idiot\"",
        "'idiot'",
        "```\nidiot\n```",
        "```\nidiot",
        "> idiot",
        "/src/idiot.rs",
        "idiot_name",
        "idiotName",
        "https://example.com/idiot",
    ] {
        let start = text.find("idiot").unwrap();
        assert_eq!(
            apply_replacements(
                text,
                &[span(start, start + 5)],
                0.5,
                &FixedMarker::default()
            )
            .unwrap_err(),
            Error::ProtectedSpan,
            "{text}"
        );
    }
}
#[test]
fn validates_boundaries_and_scores() {
    for s in [
        span(1, 2),
        span(9, 10),
        Span {
            start: 0,
            end: 4,
            score: f32::NAN,
        },
    ] {
        assert_eq!(
            apply_replacements("🙂 bad", &[s], 0.5, &FixedMarker::default()).unwrap_err(),
            Error::InvalidSpan
        );
    }
    assert_eq!(utf16_to_byte("🙂bad", 1), Err(Error::InvalidSpan));
    assert_eq!(utf16_to_byte("🙂bad", 2), Ok(4));
    assert_eq!(byte_to_utf16("🙂bad", 4), Ok(2));
}
#[test]
fn configurable_marker_and_threshold() {
    let result = apply_replacements(
        "bad phrase",
        &[span(0, 3)],
        1.0,
        &FixedMarker::new("la la").unwrap(),
    )
    .unwrap();
    assert_eq!(result.text, "bad phrase");
    assert_eq!(
        apply_replacements(
            "bad phrase",
            &[span(0, 3)],
            0.5,
            &FixedMarker::new("la la").unwrap()
        )
        .unwrap()
        .text,
        "la la phrase"
    );
    assert!(FixedMarker::new("\n").is_err());
    assert!(normalize_spans("text", &[], -0.1).is_err());
}
#[test]
fn dense_replacements_can_expand_past_the_input_limit() {
    let text = "a!".repeat(MAX_TEXT_BYTES / 2);
    let spans: Vec<_> = (0..text.len()).step_by(2).map(|i| span(i, i + 1)).collect();
    let result = apply_replacements(&text, &spans, 0.5, &FixedMarker::default()).unwrap();
    assert_eq!(result.text, "ya ya ya!".repeat(MAX_TEXT_BYTES / 2));
    assert_eq!(result.replacement_count, MAX_TEXT_BYTES / 2);
    assert_eq!(validate_text(&(text + "x")), Err(Error::InvalidInput));
}
#[test]
fn overlapping_windows_can_exceed_one_span_per_input_byte() {
    let text = "!".repeat(MAX_TEXT_BYTES);
    let spans: Vec<_> = (0..text.len())
        .map(|i| span(i, i + 1))
        .chain((0..text.len() / 3).map(|i| span(i, i + 1)))
        .collect();
    let result = apply_replacements(&text, &spans, 0.5, &FixedMarker::default()).unwrap();
    assert_eq!(result.text, DEFAULT_MARKER);
    assert_eq!(result.replacement_count, 1);
}
proptest! {
    #[test]
    fn unicode_round_trip(text in ".{0,100}") {
        for (byte, _) in text.char_indices() { prop_assert_eq!(utf16_to_byte(&text,byte_to_utf16(&text,byte).unwrap()).unwrap(),byte); }
    }
    #[test]
    fn untouched_text_is_exact(text in "[a-zA-Z ]{1,100}") {
        prop_assume!(!text.trim().is_empty());
        prop_assert_eq!(apply_replacements(&text,&[],0.5,&FixedMarker::default()).unwrap().text,text);
    }
}
