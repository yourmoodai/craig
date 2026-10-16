use pulldown_cmark::{Event, Parser, Tag};
use regex::Regex;
use std::{ops::Range, sync::LazyLock};

static LITERALS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r#""[^"\n]*"|“[^”\n]*”|'[^'\n]+'|‘[^’\n]+’|https?://[^\s<>"']+|(?:[~.]?/|[A-Za-z]:\\)[^\s,;!?"'`]+|--?[a-zA-Z][\w-]*(?:=[^\s]+)?|\b\w+(?:[._/]\w[\w-]*)+\b|\b[a-z]+(?:[A-Z][a-zA-Z0-9]*)+\b|\b[A-Z][A-Z_0-9]+\b|\b\d+(?:[.:/-]\d+)*(?:%|[a-zA-Z]+)?"#
).expect("literal pattern is valid")
});

pub fn ranges(text: &str) -> Vec<Range<usize>> {
    let mut regions: Vec<_> = Parser::new(text)
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Code(_) | Event::Start(Tag::CodeBlock(_)) | Event::Start(Tag::BlockQuote(_)) => {
                Some(range)
            }
            _ => None,
        })
        .collect();
    regions.extend(
        LITERALS
            .find_iter(text)
            .filter(|m| {
                !m.as_str().starts_with('\'')
                    || m.start() == 0
                    || !text[..m.start()]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_alphanumeric)
            })
            .map(|m| m.range()),
    );
    regions
}
