//! Pure chat transformations. Host styles are opaque and preserved on ordinary text.
use super::{
    scanner::LinkScanner,
    sources::{MediaKind, MediaRef, SourceRegistry},
};
use crate::platform::Modifiers;

/// Allocation-free fast path, including a scheme split across styled spans.
pub fn has_http<'a>(texts: impl IntoIterator<Item = &'a str>) -> bool {
    let mut matched = 0;
    for byte in texts.into_iter().flat_map(str::bytes) {
        let byte = byte.to_ascii_lowercase();
        if byte == b"http"[matched] {
            matched += 1;
            if matched == 4 {
                return true;
            }
        } else {
            matched = usize::from(byte == b'h');
        }
    }
    false
}

pub const CHAT_COMMAND: &str = "/show _chat ";
#[derive(Clone, Debug)]
pub struct ChatSpan<S> {
    pub text: String,
    pub style: S,
    /// An existing OpenUrl target (also supports labelled links).
    pub url: Option<String>,
    /// Preserve other click actions and inline objects verbatim.
    pub protected: bool,
}
#[derive(Clone, Debug)]
pub struct DisplaySpan<S> {
    pub text: String,
    pub style: S,
    pub media: Option<MediaRef>,
}

/// Scan the joined text so URLs split by formatting still form a single badge.
/// Never replace unrelated command, clipboard, or inline-object spans.
pub fn rewrite<S: Clone>(
    spans: &[ChatSpan<S>],
    sources: &SourceRegistry,
    replace: bool,
) -> Vec<DisplaySpan<S>> {
    let plain: String = spans.iter().map(|s| s.text.as_str()).collect();
    let mut offsets = Vec::with_capacity(spans.len());
    let mut offset = 0;
    for s in spans {
        offsets.push(offset);
        offset += s.text.len();
    }
    let mut links = Vec::new();
    for link in LinkScanner::scan(&plain) {
        if spans.iter().zip(&offsets).any(|(s, &start)| {
            start < link.end
                && start + s.text.len() > link.start
                && (s.protected || s.url.as_deref().is_some_and(|u| u != link.url))
        }) {
            continue;
        }
        if let Some(media) = sources.identify(&link.url) {
            links.push((link.start, link.end, media));
        }
    }
    for (s, &start) in spans.iter().zip(&offsets) {
        if !s.protected
            && !s.text.is_empty()
            && let Some(media) = s.url.as_deref().and_then(|u| sources.identify(u))
        {
            let end = start + s.text.len();
            if !links.iter().any(|(a, b, _)| *a < end && *b > start) {
                links.push((start, end, media));
            }
        }
    }
    links.sort_by_key(|l| l.0);
    let mut out = Vec::new();
    for (s, &start) in spans.iter().zip(&offsets) {
        let end = start + s.text.len();
        let mut pos = start;
        for (a, b, media) in links.iter().filter(|(a, b, _)| *a < end && *b > start) {
            let first = *a >= start;
            let a = (*a).max(start);
            if a > pos {
                out.push(DisplaySpan {
                    text: plain[pos..a].into(),
                    style: s.style.clone(),
                    media: None,
                });
            }
            let stop = (*b).min(end);
            if !replace || first {
                out.push(DisplaySpan {
                    text: if replace {
                        format!("[{}]", media.label)
                    } else {
                        plain[a..stop].into()
                    },
                    style: s.style.clone(),
                    media: Some(media.clone()),
                });
            }
            pos = stop;
        }
        if pos < end || s.text.is_empty() {
            out.push(DisplaySpan {
                text: plain[pos..end].into(),
                style: s.style.clone(),
                media: None,
            });
        }
    }
    out
}
pub fn badge_color(media: &MediaRef) -> u32 {
    match media.label.as_str() {
        "youtube" | "youtube_playlist" => 0xff5555,
        "twitch" => 0xaa55ff,
        "gif" => 0xffaa00,
        _ => match media.kind {
            MediaKind::Image => 0x55ff55,
            MediaKind::Audio => 0x55ffff,
            MediaKind::Collection => 0xffff55,
            _ => 0x5599ff,
        },
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatAction {
    Open(MediaRef),
    Enqueue(MediaRef),
    Browser(String),
    Playlists,
    RevealHidden,
}
pub fn click_action(raw: &str, mods: Modifiers, sources: &SourceRegistry) -> Option<ChatAction> {
    let media = sources.identify(raw)?;
    Some(if mods.ctrl {
        ChatAction::Browser(media.url)
    } else if mods.shift {
        ChatAction::Enqueue(media)
    } else {
        ChatAction::Open(media)
    })
}
