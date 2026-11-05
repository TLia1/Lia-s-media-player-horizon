//! `/show <type> <url> [newPlayer]`: plays a link without it being posted in chat.
//!
//! A port of the Java mod's `ShowCommand.run(type, rest)`: the link is the rest of the line
//! (so it needs no quotes, whatever characters it holds), optionally wrapped in double quotes,
//! and a trailing `true` or `false` is the `newPlayer` flag. Without `newPlayer` the link joins
//! the front player's queue (images replace the front image); `true` opens a new window.
//!
//! The parser checks grammar and HTTP(S) URLs. The image service checks formats; the
//! resolver and player actors perform page extraction and audio/video playback.

use std::fmt;

/// The command's name, without its slash.
pub const NAME: &str = "show";

/// The usage line shown when the command is malformed.
pub const USAGE: &str = "/show <image|gif|video|audio> <url> [newPlayer]";

/// How the link should be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowKind {
    Video,
    Audio,
    Image,
    Gif,
}

impl ShowKind {
    /// Every kind, in the order the command suggests them.
    pub const ALL: [ShowKind; 4] = [
        ShowKind::Video,
        ShowKind::Audio,
        ShowKind::Image,
        ShowKind::Gif,
    ];

    /// The argument's spelling.
    pub fn id(self) -> &'static str {
        match self {
            ShowKind::Video => "video",
            ShowKind::Audio => "audio",
            ShowKind::Image => "image",
            ShowKind::Gif => "gif",
        }
    }

    /// The kind `word` names, case-insensitively.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.id().eq_ignore_ascii_case(word.trim()))
    }
}

impl fmt::Display for ShowKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// A `/show` the player typed, once accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShowCommand {
    pub kind: ShowKind,
    pub url: String,
    /// `newPlayer`: open a window of its own instead of joining the front window's queue.
    pub new_player: bool,
}

/// Why a `/show` was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("Usage: {USAGE}")]
    Usage,
    #[error("Unknown type '{0}': use image, gif, video or audio")]
    InvalidType(String),
    #[error("Not a web link: {0}")]
    UnsupportedUrl(String),
}

/// Reads a command line as `native_command` gets it (without its slash).
///
/// `None` when the line is not a `/show` at all, so another mod or the server can have it;
/// `Some(Err(_))` when it is a malformed `/show`, which the mod still owns.
pub fn parse(line: &str) -> Option<Result<ShowCommand, ParseError>> {
    let line = line.trim_start();
    let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
    if name != NAME {
        return None;
    }
    Some(parse_args(rest))
}

fn parse_args(args: &str) -> Result<ShowCommand, ParseError> {
    let args = args.trim();
    let (kind, rest) = args.split_once(' ').ok_or(ParseError::Usage)?;
    let kind = ShowKind::parse(kind).ok_or_else(|| ParseError::InvalidType(kind.to_owned()))?;
    let (url, new_player) = split_new_player(rest.trim());
    let url = unquote(url).trim();
    if url.is_empty() {
        return Err(ParseError::Usage);
    }
    if !is_web_url(url) {
        return Err(ParseError::UnsupportedUrl(url.to_owned()));
    }
    Ok(ShowCommand {
        kind,
        url: url.to_owned(),
        new_player,
    })
}

/// The link and the `newPlayer` flag: a trailing `true` or `false` after a space.
fn split_new_player(rest: &str) -> (&str, bool) {
    if let Some((head, last)) = rest.rsplit_once(' ')
        && !head.trim().is_empty()
    {
        if last.eq_ignore_ascii_case("true") {
            return (head.trim_end(), true);
        }
        if last.eq_ignore_ascii_case("false") {
            return (head.trim_end(), false);
        }
    }
    (rest, false)
}

fn unquote(url: &str) -> &str {
    url.strip_prefix('"')
        .and_then(|u| u.strip_suffix('"'))
        .unwrap_or(url)
}

fn is_web_url(raw: &str) -> bool {
    crate::core::sources::web_url(raw).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(line: &str) -> ShowCommand {
        parse(line).expect("a /show").expect("accepted")
    }

    fn err(line: &str) -> ParseError {
        parse(line).expect("a /show").expect_err("refused")
    }

    #[test]
    fn other_commands_are_left_alone() {
        assert_eq!(parse("tp 0 64 0"), None);
        assert_eq!(parse("shows video https://a.b"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn accepts_each_kind() {
        for kind in ShowKind::ALL {
            let cmd = ok(&format!("show {kind} https://example.com/x"));
            assert_eq!(cmd.kind, kind);
            assert_eq!(cmd.url, "https://example.com/x");
            assert!(!cmd.new_player);
        }
    }

    #[test]
    fn kind_is_case_insensitive() {
        assert_eq!(ok("show VIDEO https://youtu.be/x").kind, ShowKind::Video);
    }

    #[test]
    fn trailing_true_or_false_is_new_player() {
        assert!(ok("show video https://youtu.be/x true").new_player);
        assert!(ok("show video https://youtu.be/x TRUE").new_player);
        assert!(!ok("show video https://youtu.be/x false").new_player);
        assert_eq!(
            ok("show video https://youtu.be/x true").url,
            "https://youtu.be/x"
        );
    }

    #[test]
    fn quoted_links_lose_their_quotes() {
        let cmd = ok("show image \"https://example.com/a b.png\" true");
        assert_eq!(cmd.url, "https://example.com/a b.png");
        assert!(cmd.new_player);
    }

    #[test]
    fn the_link_is_the_rest_of_the_line() {
        let cmd = ok("show audio https://example.com/?q=a&b=c");
        assert_eq!(cmd.url, "https://example.com/?q=a&b=c");
    }

    #[test]
    fn malformed_lines_are_refused_but_owned() {
        assert_eq!(err("show"), ParseError::Usage);
        assert_eq!(err("show video"), ParseError::Usage);
        assert_eq!(err("show video \"\""), ParseError::Usage);
        assert_eq!(
            err("show document https://a.b/c.gif"),
            ParseError::InvalidType("document".into())
        );
        assert_eq!(
            err("show video ftp://a.b/c"),
            ParseError::UnsupportedUrl("ftp://a.b/c".into())
        );
        assert_eq!(
            err("show video true"),
            ParseError::UnsupportedUrl("true".into())
        );
    }

    #[test]
    fn errors_read_well_in_chat() {
        assert_eq!(
            ParseError::Usage.to_string(),
            "Usage: /show <image|gif|video|audio> <url> [newPlayer]"
        );
    }
}
