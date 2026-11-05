use std::sync::OnceLock;

/// Offsets are UTF-8 byte offsets, suitable for slicing the original chat text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    pub start: usize,
    pub end: usize,
    pub url: String,
}
pub struct LinkScanner;
impl LinkScanner {
    pub fn scan(text: &str) -> Vec<LinkSpan> {
        static URL: OnceLock<regex::Regex> = OnceLock::new();
        URL.get_or_init(|| regex::Regex::new(r"(?i)https?://[^\s<>\x22]+").unwrap())
            .find_iter(text)
            .filter_map(|m| {
                let mut s = m.as_str();
                loop {
                    let last = s.chars().last()?;
                    if ".,;:!?'\">]}".contains(last)
                        || (last == ')' && s.matches(')').count() > s.matches('(').count())
                    {
                        s = &s[..s.len() - last.len_utf8()];
                    } else {
                        break;
                    }
                }
                super::sources::web_url(s)?;
                Some(LinkSpan {
                    start: m.start(),
                    end: m.start() + s.len(),
                    url: s.into(),
                })
            })
            .collect()
    }
}
