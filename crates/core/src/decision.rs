//! Experimental fixed-label detectors. Artifacts contain learned parameters, never input history.
use crate::{Detector, Error, Result, Span, validate_text};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use unicode_segmentation::UnicodeSegmentation;

pub const FEATURES: &str = "unicode_words_context2_char2to4_v1";
pub const LEXICAL_FEATURES: &str = "unicode_words_separated_ascii_v2";
#[derive(Clone, Debug)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub word: String,
}
pub fn tokens(text: &str) -> Vec<Token> {
    text.split_word_bound_indices()
        .filter(|(_, word)| !word.chars().all(char::is_whitespace))
        .map(|(start, word)| Token {
            start,
            end: start + word.len(),
            word: word.to_lowercase(),
        })
        .collect()
}
pub fn features(tokens: &[Token], i: usize) -> Vec<String> {
    let word = &tokens[i].word;
    let mut out = vec!["bias".into(), format!("word:{word}")];
    for delta in [-2isize, -1, 1, 2] {
        if let Some(t) = i.checked_add_signed(delta).and_then(|j| tokens.get(j)) {
            out.push(format!("context{delta}:{}", t.word));
        }
    }
    let chars: Vec<_> = word.chars().collect();
    for n in 1..=4 {
        if chars.len() >= n {
            out.push(format!(
                "prefix{n}:{}",
                chars[..n].iter().collect::<String>()
            ));
            out.push(format!(
                "suffix{n}:{}",
                chars[chars.len() - n..].iter().collect::<String>()
            ));
        }
    }
    for n in 2..=4 {
        for chars in chars.windows(n) {
            out.push(format!("char{n}:{}", chars.iter().collect::<String>()));
        }
    }
    out.push(format!(
        "shape:{}",
        if word.chars().all(char::is_alphabetic) {
            "word"
        } else if word.chars().all(char::is_numeric) {
            "number"
        } else {
            "mixed"
        }
    ));
    out
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub version: u32,
    pub kind: String,
    pub policy: String,
    pub features: String,
    pub threshold: f32,
    pub corpus_sha256: String,
    pub epoch: usize,
    pub labels_reviewed: bool,
    pub weights: BTreeMap<String, f64>,
    pub transitions: [[f64; 2]; 2],
}
impl Artifact {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !["lexical", "crf"].contains(&self.kind.as_str())
            || self.policy != "offensive_spans"
            || !(self.features == FEATURES
                || (self.kind == "lexical" && self.features == LEXICAL_FEATURES))
            || !self.threshold.is_finite()
            || self.threshold <= 0.0
            || self.threshold > 1.0
            || self.corpus_sha256.len() != 64
            || !self.corpus_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || self.weights.len() > 1_000_000
            || self.weights.iter().any(|(k, v)| {
                k.is_empty()
                    || !v.is_finite()
                    || (self.kind == "lexical" && !(0.0..=1.0).contains(v))
            })
            || self.transitions.iter().flatten().any(|v| !v.is_finite())
        {
            return Err(Error::ModelIncompatible);
        }
        Ok(())
    }
}
pub struct DecisionDetector {
    pub artifact: Artifact,
}
impl DecisionDetector {
    pub fn load(dir: impl AsRef<Path>, kind: &str) -> Result<Self> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(dir.as_ref().join("decision.json"))
            .map_err(|_| Error::ModelUnavailable)?
            .take(67_108_865)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::ModelUnavailable)?; // 64 MiB plus overflow sentinel
        if bytes.len() > 67_108_864 {
            return Err(Error::ModelIncompatible);
        } // 64 MiB
        let artifact: Artifact =
            serde_json::from_slice(&bytes).map_err(|_| Error::ModelIncompatible)?;
        artifact.validate()?;
        if artifact.kind != kind {
            return Err(Error::ModelIncompatible);
        }
        Ok(Self { artifact })
    }
    pub fn threshold(&self) -> f32 {
        self.artifact.threshold
    }
}
impl Detector for DecisionDetector {
    fn detect(&mut self, text: &str) -> Result<Vec<Span>> {
        validate_text(text)?;
        if self.artifact.kind == "lexical" {
            let mut spans = Vec::new();
            for (start, word) in text.split_word_bound_indices() {
                if word.chars().all(char::is_whitespace) {
                    continue;
                }
                let lower = if word
                    .bytes()
                    .all(|c| c.is_ascii() && !c.is_ascii_uppercase())
                {
                    std::borrow::Cow::Borrowed(word)
                } else {
                    std::borrow::Cow::Owned(word.to_lowercase())
                };
                if let Some(&score) = self.artifact.weights.get(lower.as_ref())
                    && score > 0.0
                {
                    spans.push(Span {
                        start,
                        end: start + word.len(),
                        score: score as f32,
                    });
                }
            }
            if self.artifact.features == LEXICAL_FEATURES {
                spans.extend(separated_spans(text, &self.artifact.weights));
            }
            return Ok(spans);
        }
        let tokens = tokens(text);
        let probabilities = {
            let emissions: Vec<_> = (0..tokens.len())
                .map(|i| {
                    features(&tokens, i)
                        .iter()
                        .map(|f| self.artifact.weights.get(f).copied().unwrap_or(0.0))
                        .sum()
                })
                .collect();
            infer(&emissions, &self.artifact.transitions)?.probabilities
        };
        let spans: Vec<_> = tokens
            .iter()
            .zip(probabilities)
            .filter(|(_, p)| *p > 0.0)
            .map(|(t, p)| Span {
                start: t.start,
                end: t.end,
                score: p as f32,
            })
            .collect();
        Ok(spans)
    }
}

// Only complete ASCII letter chains qualify; source spelling and offsets stay intact.
fn separated_spans(text: &str, weights: &BTreeMap<String, f64>) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        let mut word = [0u8; 33]; // 32 letters plus overflow sentinel
        word[0] = bytes[i].to_ascii_lowercase();
        let mut len = 1;
        i += 1;
        while i + 1 < bytes.len()
            && matches!(bytes[i], b'.' | b'-' | b'_' | b'*')
            && bytes[i + 1].is_ascii_alphabetic()
        {
            if len < word.len() {
                word[len] = bytes[i + 1].to_ascii_lowercase();
                len += 1;
            }
            i += 2;
        }
        let boundary = |c: char| c.is_alphanumeric() || c == '_';
        if (2..=32).contains(&len)
            && !text[..start].chars().next_back().is_some_and(boundary)
            && !text[i..].chars().next().is_some_and(boundary)
            && let Some(&score) =
                weights.get(std::str::from_utf8(&word[..len]).expect("ascii letters"))
        {
            spans.push(Span {
                start,
                end: i,
                score: score as f32,
            });
        }
    }
    spans
}
pub struct Inference {
    pub probabilities: Vec<f64>,
    pub transition_counts: [[f64; 2]; 2],
    pub log_partition: f64,
}
fn log_add(a: f64, b: f64) -> f64 {
    let m = a.max(b);
    m + ((a - m).exp() + (b - m).exp()).ln()
}
/// Exact binary-chain marginals, with state zero's emission fixed at zero.
pub fn infer(emissions: &[f64], transitions: &[[f64; 2]; 2]) -> Result<Inference> {
    if emissions
        .iter()
        .chain(transitions.iter().flatten())
        .any(|x| !x.is_finite())
    {
        return Err(Error::InferenceFailed);
    }
    let n = emissions.len();
    if n == 0 {
        return Ok(Inference {
            probabilities: vec![],
            transition_counts: [[0.; 2]; 2],
            log_partition: 0.,
        });
    }
    let mut forward = vec![[0.; 2]; n];
    forward[0] = [0., emissions[0]];
    for i in 1..n {
        for state in 0..2 {
            forward[i][state] = log_add(
                forward[i - 1][0] + transitions[0][state],
                forward[i - 1][1] + transitions[1][state],
            ) + if state == 1 { emissions[i] } else { 0. };
        }
    }
    let z = log_add(forward[n - 1][0], forward[n - 1][1]);
    if !z.is_finite() {
        return Err(Error::InferenceFailed);
    }
    let mut backward = vec![[0.; 2]; n];
    for i in (0..n - 1).rev() {
        for (state, row) in transitions.iter().enumerate() {
            backward[i][state] = log_add(
                row[0] + backward[i + 1][0],
                row[1] + emissions[i + 1] + backward[i + 1][1],
            );
        }
    }
    let probabilities = (0..n)
        .map(|i| (forward[i][1] + backward[i][1] - z).exp().clamp(0., 1.))
        .collect();
    let mut counts = [[0.; 2]; 2];
    for i in 1..n {
        for a in 0..2 {
            for b in 0..2 {
                counts[a][b] += (forward[i - 1][a]
                    + transitions[a][b]
                    + if b == 1 { emissions[i] } else { 0. }
                    + backward[i][b]
                    - z)
                    .exp();
            }
        }
    }
    Ok(Inference {
        probabilities,
        transition_counts: counts,
        log_partition: z,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marginals_and_gradients_match_exhaustive_paths() {
        let emissions = [0.2, -0.7, 1.1];
        let transitions = [[0.1, -0.2], [0.3, 0.4]];
        let got = infer(&emissions, &transitions).unwrap();
        let mut z = 0.;
        let mut marginals = [0.; 3];
        let mut counts = [[0.; 2]; 2];
        for path in 0..8 {
            let states: Vec<_> = (0..3).map(|i| (path >> i) & 1).collect();
            let score = (0..3)
                .map(|i| if states[i] == 1 { emissions[i] } else { 0. })
                .sum::<f64>()
                + (1..3)
                    .map(|i| transitions[states[i - 1]][states[i]])
                    .sum::<f64>();
            let p = score.exp();
            z += p;
            for i in 0..3 {
                marginals[i] += p * states[i] as f64;
            }
            for i in 1..3 {
                counts[states[i - 1]][states[i]] += p;
            }
        }
        assert!((got.log_partition - z.ln()).abs() < 1e-12);
        for i in 0..3 {
            assert!((got.probabilities[i] - marginals[i] / z).abs() < 1e-12);
            let mut changed = emissions;
            changed[i] += 1e-6;
            let derivative =
                (infer(&changed, &transitions).unwrap().log_partition - got.log_partition) / 1e-6;
            assert!((derivative - got.probabilities[i]).abs() < 1e-6);
        }
        for a in 0..2 {
            for b in 0..2 {
                assert!((got.transition_counts[a][b] - counts[a][b] / z).abs() < 1e-12);
                let mut changed = transitions;
                changed[a][b] += 1e-6;
                let derivative =
                    (infer(&emissions, &changed).unwrap().log_partition - got.log_partition) / 1e-6;
                assert!((derivative - got.transition_counts[a][b]).abs() < 1e-6);
            }
        }
    }
    #[test]
    fn artifacts_round_trip_and_reject_invalid_parameters() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = Artifact {
            version: 1,
            kind: "lexical".into(),
            policy: "offensive_spans".into(),
            features: FEATURES.into(),
            threshold: 0.5,
            corpus_sha256: "a".repeat(64),
            epoch: 0,
            labels_reviewed: false,
            weights: BTreeMap::from([("idiot".into(), 0.9)]),
            transitions: [[0.; 2]; 2],
        };
        std::fs::write(
            dir.path().join("decision.json"),
            serde_json::to_vec(&artifact).unwrap(),
        )
        .unwrap();
        let mut detector = DecisionDetector::load(dir.path(), "lexical").unwrap();
        let text = "🙂 You idiot, preserve café.";
        let result =
            crate::rewrite(&mut detector, text, 0.5, &crate::FixedMarker::default()).unwrap();
        assert_eq!(result.text, "🙂 You ya ya ya, preserve café.");
        assert_eq!(result.spans[0].start, 9);
        assert!(DecisionDetector::load(dir.path(), "crf").is_err());
        let mut invalid = artifact;
        invalid.transitions[0][1] = f64::NAN;
        assert!(invalid.validate().is_err());
        assert!(infer(&[f64::INFINITY], &[[0.; 2]; 2]).is_err());
    }
    #[test]
    fn offsets_cover_unicode_and_punctuation() {
        let text = "🙂 CAFÉ!";
        let t = tokens(text);
        assert_eq!(
            t.iter().map(|t| &text[t.start..t.end]).collect::<Vec<_>>(),
            vec!["🙂", "CAFÉ", "!"]
        );
        assert_eq!(t[1].word, "café");
        assert_eq!(tokens(&"!".repeat(65536)).len(), 65536);
    }
}
