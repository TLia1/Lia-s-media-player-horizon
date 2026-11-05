use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Collection,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
    pub url: String,
    pub kind: MediaKind,
    pub source_id: String,
    pub label: String,
    pub entry_kind: MediaKind,
}
pub fn web_url(raw: &str) -> Option<Url> {
    if raw
        .chars()
        .any(|c| (c.is_whitespace() && c != ' ') || c.is_control())
    {
        return None;
    }
    let u = Url::parse(raw).ok()?;
    (matches!(u.scheme(), "http" | "https")
        && u.host_str().is_some()
        && u.username().is_empty()
        && u.password().is_none())
    .then_some(u)
}
pub trait LinkSource: Send + Sync {
    fn id(&self) -> &str;
    fn priority(&self) -> i32 {
        0
    }
    fn matches(&self, url: &Url) -> Option<(MediaKind, MediaKind, &str)>;
}
pub struct SourceRegistry {
    sources: Vec<Box<dyn LinkSource>>,
}
impl Default for SourceRegistry {
    fn default() -> Self {
        let mut r = Self {
            sources: Vec::new(),
        };
        for name in [
            "picture",
            "tenor",
            "giphy",
            "youtube_playlist",
            "youtube",
            "twitch",
            "vimeo",
            "streamable",
            "reddit",
            "soundcloud_set",
            "soundcloud",
            "bandcamp_album",
            "bandcamp",
            "video_file",
            "audio_file",
            "stream",
        ] {
            r.register(Box::new(Builtin {
                name,
                id: format!("liasmediaplayer:{name}"),
            }));
        }
        r
    }
}
impl SourceRegistry {
    pub fn register(&mut self, source: Box<dyn LinkSource>) {
        self.sources.push(source);
        self.sources
            .sort_by_key(|s| std::cmp::Reverse(s.priority()));
    }
    pub fn sources(&self) -> &[Box<dyn LinkSource>] {
        &self.sources
    }
    pub fn identify(&self, raw: &str) -> Option<MediaRef> {
        let url = web_url(raw)?;
        // Snapshot addon registrations before calling user code; equal-priority addons win.
        let addons = crate::api::sources();
        if addons.is_empty() {
            return self.sources.iter().find_map(|s| {
                s.matches(&url).map(|(kind, entry_kind, label)| MediaRef {
                    url: raw.into(),
                    kind,
                    entry_kind,
                    source_id: s.id().into(),
                    label: label.into(),
                })
            });
        }
        let mut sources: Vec<&dyn LinkSource> = addons
            .iter()
            .map(|s| s.as_ref())
            .chain(self.sources.iter().map(|s| s.as_ref()))
            .collect();
        sources.sort_by_key(|s| std::cmp::Reverse(s.priority()));
        sources.into_iter().find_map(|s| {
            s.matches(&url).map(|(kind, entry_kind, label)| MediaRef {
                url: raw.into(),
                kind,
                entry_kind,
                source_id: s.id().into(),
                label: label.into(),
            })
        })
    }
}
struct Builtin {
    name: &'static str,
    id: String,
}
impl LinkSource for Builtin {
    fn id(&self) -> &str {
        &self.id
    }
    fn priority(&self) -> i32 {
        if matches!(
            self.name,
            "youtube_playlist" | "soundcloud_set" | "bandcamp_album"
        ) {
            10
        } else {
            0
        }
    }
    fn matches(&self, u: &Url) -> Option<(MediaKind, MediaKind, &str)> {
        use MediaKind::*;
        let h = u.host_str()?.strip_prefix("www.").unwrap_or(u.host_str()?);
        let p = u.path().to_ascii_lowercase();
        let seg: Vec<_> = p.split('/').filter(|s| !s.is_empty()).collect();
        let ext = p.rsplit('.').next().unwrap_or("");
        let yt = ["youtube.com", "m.youtube.com", "music.youtube.com"].contains(&h);
        let sc = [
            "soundcloud.com",
            "m.soundcloud.com",
            "on.soundcloud.com",
            "snd.sc",
        ]
        .contains(&h);
        let bc = h == "bandcamp.com" || h.ends_with(".bandcamp.com");
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        let (yes, kind, entry, label) = match self.name {
            "picture" => (
                ["png", "jpg", "jpeg", "gif", "bmp", "webp", "avif", "apng"].contains(&ext),
                Image,
                Image,
                if ext == "gif" { "gif" } else { "picture" },
            ),
            "tenor" => (
                h == "tenor.com" && p.contains("/view/"),
                Image,
                Image,
                "gif",
            ),
            "giphy" => (
                h == "giphy.com"
                    && seg.len() >= 2
                    && ["gifs", "clips", "stickers"].contains(&seg[0])
                    && seg[1].rsplit('-').next().is_some_and(|id| {
                        id.len() >= 8 && id.bytes().all(|b| b.is_ascii_alphanumeric())
                    }),
                Image,
                Image,
                "gif",
            ),
            "youtube_playlist" => (
                (yt || h == "youtu.be")
                    && (p == "/playlist" || p == "/watch" || h == "youtu.be")
                    && u.query_pairs().any(|(k, v)| k == "list" && !v.is_empty()),
                Collection,
                Video,
                "youtube_playlist",
            ),
            "youtube" => (
                (h == "youtu.be" && !seg.is_empty())
                    || (yt
                        && (p == "/watch"
                            || ["/shorts/", "/embed/", "/live/"]
                                .iter()
                                .any(|s| p.starts_with(s)))),
                Video,
                Video,
                "youtube",
            ),
            "twitch" => (
                ["twitch.tv", "m.twitch.tv", "clips.twitch.tv"].contains(&h) && !seg.is_empty(),
                Video,
                Video,
                "twitch",
            ),
            "vimeo" => (
                (h == "vimeo.com" && seg.first().is_some_and(|s| digits(s)))
                    || (h == "player.vimeo.com"
                        && seg.len() >= 2
                        && seg[0] == "video"
                        && digits(seg[1])),
                Video,
                Video,
                "vimeo",
            ),
            "streamable" => (
                h == "streamable.com"
                    && seg.last().is_some_and(|s| {
                        s.len() <= 12
                            && s.bytes().all(|b| b.is_ascii_alphanumeric())
                            && !["login", "signup", "pricing", "terms"].contains(s)
                    }),
                Video,
                Video,
                "streamable",
            ),
            "reddit" => (
                (h == "v.redd.it" && !seg.is_empty())
                    || ([
                        "reddit.com",
                        "old.reddit.com",
                        "new.reddit.com",
                        "m.reddit.com",
                    ]
                    .contains(&h)
                        && p.contains("/comments/")),
                Video,
                Video,
                "reddit",
            ),
            "soundcloud_set" => (
                sc && seg.len() >= 3 && seg[1] == "sets",
                Collection,
                Audio,
                "soundcloud_set",
            ),
            "soundcloud" => (sc && !seg.is_empty(), Audio, Audio, "soundcloud"),
            "bandcamp_album" => (
                bc && p.starts_with("/album/"),
                Collection,
                Audio,
                "bandcamp_album",
            ),
            "bandcamp" => (bc && p.starts_with("/track/"), Audio, Audio, "bandcamp"),
            "video_file" => (
                [
                    "mp4", "webm", "mov", "mkv", "m4v", "avi", "flv", "ogv", "ts",
                ]
                .contains(&ext),
                Video,
                Video,
                "video",
            ),
            "audio_file" => (
                [
                    "mp3", "wav", "ogg", "oga", "flac", "m4a", "aac", "opus", "weba", "wma",
                    "aiff", "aif",
                ]
                .contains(&ext),
                Audio,
                Audio,
                "audio",
            ),
            "stream" => (["m3u8", "mpd"].contains(&ext), Video, Video, "stream"),
            _ => return None,
        };
        yes.then_some((kind, entry, label))
    }
}
