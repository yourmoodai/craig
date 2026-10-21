//! Local span detection and deterministic replacement, independent of agent runtimes.
mod artifact;
mod bert;
mod coreml;
pub mod decision;
mod model;
pub mod model_catalog;
mod runtime;
pub use artifact::{ModelManifest, Policy, artifact_digest};
pub use coreml::ComputeUnits;
pub use runtime::RuntimeDetector;
mod protected;
pub use model::{Backend, ExecutionOptions, InferenceTimings, ModelDetector};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_TEXT_BYTES: usize = 65_536; // 64 KiB
pub const DEFAULT_MARKER: &str = "ya ya ya";
pub const MODEL_ID: &str = "mudes/en-base";
pub const MODEL_REVISION: &str = "1600298e0458d961c2c749da7a098f69b4be478a";

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum Error {
    #[error("invalid_input")]
    InvalidInput,
    #[error("invalid_span")]
    InvalidSpan,
    #[error("invalid_options")]
    InvalidOptions,
    #[error("protected_span_detected")]
    ProtectedSpan,
    #[error("model_unavailable")]
    ModelUnavailable,
    #[error("model_incompatible")]
    ModelIncompatible,
    #[error("inference_failed")]
    InferenceFailed,
    #[error("processing_timeout")]
    Timeout,
    #[error("busy")]
    Busy,
    #[error("policy_unavailable")]
    PolicyUnavailable,
}
pub type Result<T> = std::result::Result<T, Error>;

/// Source offsets are UTF-8 bytes; JavaScript bindings convert them to UTF-16 units.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub score: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RewriteResult {
    pub text: String,
    pub replacement_count: usize,
    pub spans: Vec<Span>,
}

pub trait Detector {
    fn detect(&mut self, text: &str) -> Result<Vec<Span>>;
}
pub trait ReplacementStrategy {
    fn replacement(&self, span: &Span) -> Result<String>;
}

#[derive(Clone)]
pub enum ReplacementMode {
    FixedPhrase(FixedMarker),
    Omit,
}
impl ReplacementMode {
    pub fn new(strategy: Option<&str>, marker: Option<String>) -> Result<Self> {
        match strategy.unwrap_or("fixed_phrase") {
            "fixed_phrase" => Ok(Self::FixedPhrase(FixedMarker::new(
                marker.unwrap_or_else(|| DEFAULT_MARKER.into()),
            )?)),
            "omit" if marker.is_none() => Ok(Self::Omit),
            "omit" => Err(Error::InvalidOptions),
            _ => Err(Error::InvalidOptions),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::FixedPhrase(_) => "fixed_phrase",
            Self::Omit => "omit",
        }
    }

    pub fn marker(&self) -> Option<&str> {
        match self {
            Self::FixedPhrase(marker) => Some(&marker.0),
            Self::Omit => None,
        }
    }
}
impl Default for ReplacementMode {
    fn default() -> Self {
        Self::FixedPhrase(FixedMarker::default())
    }
}
impl ReplacementStrategy for ReplacementMode {
    fn replacement(&self, span: &Span) -> Result<String> {
        match self {
            Self::FixedPhrase(marker) => marker.replacement(span),
            Self::Omit => Ok(String::new()),
        }
    }
}

#[derive(Clone)]
pub struct FixedMarker(String);
impl Default for FixedMarker {
    fn default() -> Self {
        Self(DEFAULT_MARKER.into())
    }
}
impl FixedMarker {
    pub fn new(marker: impl Into<String>) -> Result<Self> {
        let marker = marker.into();
        if marker.trim().is_empty()
            || marker.chars().count() > 80
            || marker.chars().any(char::is_control)
        {
            return Err(Error::InvalidOptions);
        }
        Ok(Self(marker))
    }
}
impl ReplacementStrategy for FixedMarker {
    fn replacement(&self, _: &Span) -> Result<String> {
        Ok(self.0.clone())
    }
}

pub fn validate_text(text: &str) -> Result<()> {
    if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

/// Replacement spans cover whole words; any protected-text overlap rejects the entire edit.
pub fn normalize_spans(text: &str, spans: &[Span], threshold: f32) -> Result<Vec<Span>> {
    validate_text(text)?;
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(Error::InvalidOptions);
    }
    // Overlapping model windows can emit more spans than input bytes.
    if spans.len() > MAX_TEXT_BYTES * 2 {
        return Err(Error::InvalidSpan);
    }
    let words: Vec<Range<usize>> = text
        .split_word_bound_indices()
        .filter(|(_, word)| word.chars().any(|c| c.is_alphanumeric() || c == '_'))
        .map(|(start, word)| start..start + word.len())
        .collect();
    let mut selected = Vec::new();
    for span in spans {
        if span.start >= span.end
            || span.end > text.len()
            || !text.is_char_boundary(span.start)
            || !text.is_char_boundary(span.end)
            || !span.score.is_finite()
            || !(0.0..=1.0).contains(&span.score)
        {
            return Err(Error::InvalidSpan);
        }
        if span.score < threshold {
            continue;
        }
        let mut expanded = span.clone();
        let first = words.partition_point(|word| word.end <= span.start);
        let end = words.partition_point(|word| word.start < span.end);
        if first < end {
            expanded.start = expanded.start.min(words[first].start);
            expanded.end = expanded.end.max(words[end - 1].end);
        }
        selected.push(expanded);
    }
    selected.sort_by_key(|s| (s.start, s.end));
    let mut merged: Vec<Span> = Vec::new();
    for span in selected {
        if let Some(last) = merged.last_mut()
            && (span.start <= last.end
                || text[last.end..span.start].chars().all(char::is_whitespace))
        {
            last.end = last.end.max(span.end);
            last.score = last.score.max(span.score);
            continue;
        }
        merged.push(span);
    }
    let mut protected = protected::ranges(text);
    protected.sort_by_key(|range| range.start);
    let mut prefix_end = 0;
    let protected: Vec<_> = protected
        .into_iter()
        .map(|range| {
            prefix_end = prefix_end.max(range.end);
            (range.start, prefix_end)
        })
        .collect();
    if merged.iter().any(|s| {
        let end = protected.partition_point(|&(start, _)| start < s.end);
        end > 0 && protected[end - 1].1 > s.start
    }) {
        return Err(Error::ProtectedSpan);
    }
    Ok(merged)
}

pub fn apply_replacements(
    text: &str,
    spans: &[Span],
    threshold: f32,
    strategy: &dyn ReplacementStrategy,
) -> Result<RewriteResult> {
    let spans = normalize_spans(text, spans, threshold)?;
    let mut markers = Vec::with_capacity(spans.len());
    let mut output_bytes = text.len();
    for span in &spans {
        let marker = strategy.replacement(span)?;
        if !marker.is_empty() {
            FixedMarker::new(&marker)?;
        }
        output_bytes = output_bytes
            .checked_sub(span.end - span.start)
            .and_then(|n| n.checked_add(marker.len()))
            .ok_or(Error::InvalidInput)?;
        markers.push(marker);
    }
    // Validated, nonoverlapping spans and bounded markers determine the allocation.
    let mut output = String::with_capacity(output_bytes);
    let mut cursor = 0;
    for (span, marker) in spans.iter().zip(markers) {
        output.push_str(&text[cursor..span.start]);
        output.push_str(&marker);
        cursor = span.end;
    }
    output.push_str(&text[cursor..]);
    debug_assert_eq!(output.len(), output_bytes);
    Ok(RewriteResult {
        text: output,
        replacement_count: spans.len(),
        spans,
    })
}

pub fn rewrite(
    detector: &mut dyn Detector,
    text: &str,
    threshold: f32,
    strategy: &dyn ReplacementStrategy,
) -> Result<RewriteResult> {
    let spans = detector.detect(text)?;
    apply_replacements(text, &spans, threshold, strategy)
}

pub fn byte_to_utf16(text: &str, offset: usize) -> Result<u32> {
    if !text.is_char_boundary(offset) {
        return Err(Error::InvalidSpan);
    }
    u32::try_from(text[..offset].encode_utf16().count()).map_err(|_| Error::InvalidSpan)
}
pub fn utf16_to_byte(text: &str, offset: u32) -> Result<usize> {
    let mut units = 0;
    for (byte, c) in text.char_indices() {
        if units == offset {
            return Ok(byte);
        }
        units += c.len_utf16() as u32;
        if units > offset {
            return Err(Error::InvalidSpan);
        }
    }
    if units == offset {
        Ok(text.len())
    } else {
        Err(Error::InvalidSpan)
    }
}

#[cfg(test)]
mod normalization_parity {
    use super::*;
    use proptest::prelude::*;
    // Brute-force oracle preserves the pre-indexing overlap semantics.
    fn reference_normalize(text: &str, spans: &[Span], threshold: f32) -> Result<Vec<Span>> {
        validate_text(text)?;
        if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
            return Err(Error::InvalidOptions);
        }
        if spans.len() > MAX_TEXT_BYTES * 2 {
            return Err(Error::InvalidSpan);
        }
        let words: Vec<Range<usize>> = text
            .split_word_bound_indices()
            .filter(|(_, word)| word.chars().any(|c| c.is_alphanumeric() || c == '_'))
            .map(|(start, word)| start..start + word.len())
            .collect();
        let mut selected = Vec::new();
        for span in spans {
            if span.start >= span.end
                || span.end > text.len()
                || !text.is_char_boundary(span.start)
                || !text.is_char_boundary(span.end)
                || !span.score.is_finite()
                || !(0.0..=1.0).contains(&span.score)
            {
                return Err(Error::InvalidSpan);
            }
            if span.score < threshold {
                continue;
            }
            let mut expanded = span.clone();
            for word in &words {
                if word.start < span.end && word.end > span.start {
                    expanded.start = expanded.start.min(word.start);
                    expanded.end = expanded.end.max(word.end);
                }
            }
            selected.push(expanded);
        }
        selected.sort_by_key(|s| (s.start, s.end));
        let mut merged: Vec<Span> = Vec::new();
        for span in selected {
            if let Some(last) = merged.last_mut()
                && (span.start <= last.end
                    || text[last.end..span.start].chars().all(char::is_whitespace))
            {
                last.end = last.end.max(span.end);
                last.score = last.score.max(span.score);
                continue;
            }
            merged.push(span);
        }
        let protected = protected::ranges(text);
        if merged
            .iter()
            .any(|s| protected.iter().any(|p| s.start < p.end && s.end > p.start))
        {
            return Err(Error::ProtectedSpan);
        }
        Ok(merged)
    }

    proptest! {
        #[test]
        fn indexed_ranges_match_brute_force(text in "[a-zA-Z0-9 _`'\"/🙂é\n.-]{1,100}", pairs in prop::collection::vec((any::<usize>(), any::<usize>(), 0u8..=100), 0..80)) {
            prop_assume!(!text.trim().is_empty());
            let mut boundaries: Vec<_> = text.char_indices().map(|(i,_)|i).collect(); boundaries.push(text.len());
            let spans: Vec<_> = pairs.into_iter().filter_map(|(a,b,score)| {
                let (a,b) = (boundaries[a % boundaries.len()], boundaries[b % boundaries.len()]);
                (a!=b).then_some(Span{ start:a.min(b),end:a.max(b),score:f32::from(score)/100.0 })
            }).collect();
            prop_assert_eq!(normalize_spans(&text,&spans,0.5),reference_normalize(&text,&spans,0.5));
        }
    }
}
