//! Blocking URL resolution services. Owners schedule these on background workers.
use super::{
    process::{Cancellation, ProcessRunner},
    sources::{MediaKind, MediaRef, SourceRegistry, web_url},
    tools::{Tool, ToolManager},
};
use parking_lot::Mutex;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    io::{self, Read},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResolveOptions {
    pub max_height: u32,
    pub audio_only: bool,
}
impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            max_height: 720,
            audio_only: false,
        }
    }
}
#[derive(Debug, Clone)]
pub struct StreamUrl {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}
#[derive(Debug, Clone)]
pub struct ResolvedMedia {
    pub source_url: String,
    pub webpage_url: Option<String>,
    pub kind: MediaKind,
    pub video: Option<StreamUrl>,
    pub audio: Option<StreamUrl>,
    pub title: Option<String>,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
    pub is_live: bool,
    pub expires_at: SystemTime,
}
impl ResolvedMedia {
    fn direct(url: &str, kind: MediaKind, now: SystemTime) -> Self {
        let stream = Some(StreamUrl {
            url: url.into(),
            headers: BTreeMap::new(),
        });
        let mut media = Self {
            source_url: url.into(),
            webpage_url: None,
            kind,
            video: None,
            audio: None,
            title: None,
            thumbnail: None,
            duration: None,
            is_live: false,
            expires_at: now + Duration::from_secs(3600),
        };
        if kind == MediaKind::Audio {
            media.audio = stream;
        } else {
            media.video = stream;
        }
        media.update_expiry(now);
        media
    }
    fn update_expiry(&mut self, now: SystemTime) {
        self.expires_at = self
            .video
            .iter()
            .chain(self.audio.iter())
            .filter_map(|s| url_expiry(&s.url))
            .min()
            .unwrap_or(now + Duration::from_secs(3600));
    }
}
pub fn url_expiry(raw: &str) -> Option<SystemTime> {
    let url = web_url(raw)?;
    let query = url
        .query_pairs()
        .filter(|(k, _)| k == "expire" || k == "expires")
        .filter_map(|(_, v)| v.parse::<u64>().ok())
        .min();
    let segments: Vec<_> = url.path_segments()?.collect();
    let path = segments
        .windows(2)
        .filter(|p| p[0] == "expire")
        .filter_map(|p| p[1].parse::<u64>().ok())
        .min();
    query
        .into_iter()
        .chain(path)
        .min()
        .and_then(|s| UNIX_EPOCH.checked_add(Duration::from_secs(s)))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolveKey {
    pub url: String,
    pub options: ResolveOptions,
}
struct Cached {
    media: ResolvedMedia,
    used: u64,
}
struct CacheState {
    entries: HashMap<ResolveKey, Cached>,
    clock: u64,
}
pub struct ResolveCache {
    state: Mutex<CacheState>,
    capacity: usize,
}
impl ResolveCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            state: Mutex::new(CacheState {
                entries: HashMap::new(),
                clock: 0,
            }),
        }
    }
    pub fn get(&self, key: &ResolveKey, now: SystemTime) -> Option<ResolvedMedia> {
        let mut state = self.state.lock();
        state.entries.retain(|_, c| c.media.expires_at > now);
        state.clock += 1;
        let clock = state.clock;
        let cached = state.entries.get_mut(key)?;
        cached.used = clock;
        Some(cached.media.clone())
    }
    pub fn insert(&self, key: ResolveKey, media: ResolvedMedia, now: SystemTime) {
        if self.capacity == 0 || media.expires_at <= now {
            return;
        }
        let mut state = self.state.lock();
        state.entries.retain(|_, c| c.media.expires_at > now);
        state.clock += 1;
        let used = state.clock;
        state.entries.insert(key, Cached { media, used });
        while state.entries.len() > self.capacity {
            let oldest = state
                .entries
                .iter()
                .min_by_key(|(_, c)| c.used)
                .map(|(k, _)| k.clone())
                .unwrap();
            state.entries.remove(&oldest);
        }
    }
    pub fn invalidate(&self, url: &str) {
        self.state.lock().entries.retain(|k, _| k.url != url);
    }
    pub fn on_http_status(&self, url: &str, status: u16) {
        if status == 403 {
            self.invalidate(url);
        }
    }
}

pub struct DirectResolver;
impl DirectResolver {
    pub fn resolve(raw: &str, kind: MediaKind) -> io::Result<ResolvedMedia> {
        web_url(raw).ok_or_else(|| io::Error::other("Expected an HTTP(S) media URL"))?;
        if kind == MediaKind::Collection {
            return Err(io::Error::other("Expand collections before resolving"));
        }
        Ok(ResolvedMedia::direct(raw, kind, SystemTime::now()))
    }
}
pub struct GiphyResolver;
impl GiphyResolver {
    pub fn resolve(raw: &str) -> io::Result<ResolvedMedia> {
        let url = web_url(raw).ok_or_else(|| io::Error::other("Invalid Giphy URL"))?;
        if !matches!(url.host_str(), Some("giphy.com" | "www.giphy.com")) {
            return Err(io::Error::other("Not a Giphy page"));
        }
        let mut parts = url.path_segments().unwrap();
        if !matches!(parts.next(), Some("gifs" | "stickers" | "clips")) {
            return Err(io::Error::other("Not a Giphy media page"));
        }
        let id = parts.next().unwrap_or("").rsplit('-').next().unwrap_or("");
        if id.len() < 8 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(io::Error::other("Invalid Giphy id"));
        }
        let mut media = DirectResolver::resolve(
            &format!("https://media.giphy.com/media/{id}/giphy.gif"),
            MediaKind::Image,
        )?;
        media.source_url = raw.into();
        Ok(media)
    }
}
pub struct TenorResolver;
impl TenorResolver {
    pub fn resolve(raw: &str, cancel: &Cancellation) -> io::Result<ResolvedMedia> {
        let u = web_url(raw).ok_or_else(|| io::Error::other("Invalid Tenor URL"))?;
        if !matches!(u.host_str(), Some("tenor.com" | "www.tenor.com")) {
            return Err(io::Error::other("Not a Tenor page"));
        }
        if cancel.is_cancelled() {
            return Err(io::Error::other("Resolution cancelled"));
        }
        let response = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .redirects(5)
            .build()
            .get(raw)
            .set("User-Agent", "LiasMediaPlayer/4.0")
            .call()
            .map_err(io::Error::other)?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(512 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 512 * 1024 {
            return Err(io::Error::other("Tenor page exceeds 512 KiB"));
        }
        let gif = Self::gif_of(&String::from_utf8_lossy(&bytes))
            .ok_or_else(|| io::Error::other("No GIF found on Tenor page"))?;
        let mut media = DirectResolver::resolve(&gif, MediaKind::Image)?;
        media.source_url = raw.into();
        Ok(media)
    }
    pub fn gif_of(html: &str) -> Option<String> {
        let html = html
            .replace("\\/", "/")
            .replace("&amp;", "&")
            .replace("&#x2F;", "/")
            .replace("&#47;", "/");
        let base = r"https?://(?:media\d*|c)\.tenor\.com/m/([A-Za-z0-9_-]{8,})/";
        for pattern in [
            format!(r#"(?i)<img[^>]+src\s*=\s*["']{base}[^"']+\.gif["']"#),
            format!(r#"(?i)itemprop\s*=\s*["']contentUrl["'][^>]*content\s*=\s*["']{base}"#),
            format!(
                r#"(?i)content\s*=\s*["']{base}[^"']+["'][^>]*itemprop\s*=\s*["']contentUrl["']"#
            ),
            base.into(),
        ] {
            if let Some(c) = regex::Regex::new(&pattern).ok()?.captures(&html) {
                return Some(format!("https://c.tenor.com/{}/tenor.gif", &c[1]));
            }
        }
        let tags = regex::Regex::new(r"(?i)<meta\s+[^>]*>").ok()?;
        let attr = regex::Regex::new(r#"(?i)(property|content)\s*=\s*["']([^"']*)["']"#).ok()?;
        for tag in tags.find_iter(&html) {
            let attrs: HashMap<_, _> = attr
                .captures_iter(tag.as_str())
                .map(|c| (c[1].to_ascii_lowercase(), c[2].to_owned()))
                .collect();
            if attrs
                .get("property")
                .is_some_and(|v| v.eq_ignore_ascii_case("og:image"))
                && let Some(value) = attrs.get("content")
                && web_url(value).is_some_and(|u| u.path().to_ascii_lowercase().ends_with(".gif"))
            {
                return Some(value.clone());
            }
        }
        None
    }
}

pub struct YtDlpResolver;
impl YtDlpResolver {
    pub fn command(path: &std::path::Path, raw: &str, options: ResolveOptions) -> Command {
        let mut command = Command::new(path);
        let h = options.max_height.clamp(144, 4320);
        let format = if options.audio_only {
            "ba/b".into()
        } else {
            format!("bv*[height<={h}]+ba/b[height<={h}]/b")
        };
        command.args([
            "--ignore-config",
            "--no-playlist",
            "--no-warnings",
            "--no-progress",
            "--skip-download",
            "-J",
            "-f",
            &format,
            "--",
            raw,
        ]);
        command
    }
    pub fn resolve(
        tools: &ToolManager,
        raw: &str,
        kind: MediaKind,
        mut options: ResolveOptions,
        cancel: &Cancellation,
    ) -> io::Result<ResolvedMedia> {
        web_url(raw).ok_or_else(|| io::Error::other("Expected an HTTP(S) URL"))?;
        options.audio_only |= kind == MediaKind::Audio;
        let path = tools.require(Tool::YtDlp, cancel)?;
        let bytes = ProcessRunner::run(
            &mut Self::command(&path, raw, options),
            Duration::from_secs(60),
            32 * 1024 * 1024,
            cancel,
        )?;
        Self::parse(&bytes, raw, kind, SystemTime::now())
    }
    pub fn parse(
        bytes: &[u8],
        source: &str,
        kind: MediaKind,
        now: SystemTime,
    ) -> io::Result<ResolvedMedia> {
        let root: Value = serde_json::from_slice(bytes).map_err(io::Error::other)?;
        if !root.is_object() || root.get("entries").is_some() {
            return Err(io::Error::other("Expected one resolved media item"));
        }
        let common = headers(&root["http_headers"])?;
        let mut media = ResolvedMedia::direct(source, kind, now);
        media.video = None;
        media.audio = None;
        let formats = root["requested_formats"].as_array();
        if let Some(formats) = formats.filter(|f| !f.is_empty()) {
            for format in formats {
                let stream = parse_stream(format, &common)?;
                if format["vcodec"].as_str().is_some_and(|c| c != "none") {
                    media.video = Some(stream);
                } else if format["acodec"].as_str().is_some_and(|c| c != "none") {
                    media.audio = Some(stream);
                }
            }
        } else {
            let stream = parse_stream(&root, &common)?;
            if kind == MediaKind::Audio || root["vcodec"] == "none" {
                media.audio = Some(stream);
            } else {
                media.video = Some(stream);
            }
        }
        if media.video.is_none() && media.audio.is_none() {
            return Err(io::Error::other(
                "yt-dlp returned no playable HTTP(S) stream",
            ));
        }
        media.webpage_url = root["webpage_url"]
            .as_str()
            .filter(|s| web_url(s).is_some())
            .map(str::to_owned);
        media.title = root["title"].as_str().map(str::to_owned);
        media.thumbnail = root["thumbnail"]
            .as_str()
            .filter(|s| web_url(s).is_some())
            .map(str::to_owned);
        media.duration = root["duration"]
            .as_f64()
            .filter(|d| d.is_finite() && *d >= 0.);
        media.is_live =
            root["is_live"].as_bool().unwrap_or(false) || root["live_status"] == "is_live";
        media.update_expiry(now);
        Ok(media)
    }
}
fn headers(value: &Value) -> io::Result<BTreeMap<String, String>> {
    let mut headers = BTreeMap::new();
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            let value = value
                .as_str()
                .ok_or_else(|| io::Error::other("Invalid stream header"))?;
            if key.is_empty()
                || !key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                || value.chars().any(char::is_control)
            {
                return Err(io::Error::other("Invalid stream header"));
            }
            headers.insert(key.clone(), value.into());
        }
    }
    Ok(headers)
}
fn parse_stream(value: &Value, common: &BTreeMap<String, String>) -> io::Result<StreamUrl> {
    let url = value["url"]
        .as_str()
        .filter(|s| web_url(s).is_some())
        .ok_or_else(|| io::Error::other("Missing HTTP(S) stream URL"))?;
    let mut merged = common.clone();
    merged.extend(headers(&value["http_headers"])?);
    Ok(StreamUrl {
        url: url.into(),
        headers: merged,
    })
}

pub struct ResolverChain {
    pub tools: ToolManager,
    pub cache: ResolveCache,
    sources: SourceRegistry,
}
impl ResolverChain {
    pub fn new(tools: ToolManager) -> Self {
        Self {
            tools,
            cache: ResolveCache::new(128),
            sources: SourceRegistry::default(),
        }
    }
    pub fn resolve(
        &self,
        raw: &str,
        kind: MediaKind,
        options: ResolveOptions,
        cancel: &Cancellation,
    ) -> io::Result<ResolvedMedia> {
        if cancel.is_cancelled() {
            return Err(io::Error::other("Resolution cancelled"));
        }
        web_url(raw).ok_or_else(|| io::Error::other("Expected an HTTP(S) URL"))?;
        let identified = self.sources.identify(raw);
        let kind = if kind == MediaKind::Audio
            && identified
                .as_ref()
                .is_none_or(|s| s.kind != MediaKind::Collection)
        {
            MediaKind::Audio
        } else {
            identified.as_ref().map_or(kind, |s| s.kind)
        };
        if kind == MediaKind::Collection {
            return Err(io::Error::other(
                "Use CollectionExpander for collection URLs",
            ));
        }
        let options = ResolveOptions {
            audio_only: options.audio_only || kind == MediaKind::Audio,
            ..options
        };
        // Addons run before the built-in cache, so late registrations take effect on
        // existing players. Addons own caching of their results and expiration policy.
        for resolver in crate::api::resolvers() {
            if let Some(result) = resolver.resolve(
                crate::api::ResolveRequest {
                    url: raw,
                    kind,
                    options,
                },
                cancel,
            ) {
                let mut media = result?;
                if cancel.is_cancelled() {
                    return Err(io::Error::other("Resolution cancelled"));
                }
                if media.kind != kind || media.video.is_none() && media.audio.is_none() {
                    return Err(io::Error::other("Addon returned mismatched or empty media"));
                }
                for stream in media.video.iter().chain(media.audio.iter()) {
                    web_url(&stream.url)
                        .ok_or_else(|| io::Error::other("Addon returned a non-HTTP(S) stream"))?;
                    if stream.headers.iter().any(|(k, v)| {
                        k.is_empty() || k.contains(['\r', '\n', ':']) || v.contains(['\r', '\n'])
                    }) {
                        return Err(io::Error::other("Addon returned invalid HTTP headers"));
                    }
                }
                if media.duration.is_some_and(|d| !d.is_finite() || d < 0.) {
                    return Err(io::Error::other("Addon returned an invalid duration"));
                }
                media.source_url = raw.to_owned();
                return Ok(media);
            }
        }
        // Include media intent for extensionless direct URLs in the cache identity.
        let key = ResolveKey {
            url: format!("{kind:?}:{raw}"),
            options,
        };
        if let Some(media) = self.cache.get(&key, SystemTime::now()) {
            return Ok(media);
        }
        let id = identified
            .as_ref()
            .map(|s| s.source_id.as_str())
            .unwrap_or("");
        let media = if id == "liasmediaplayer:giphy" {
            GiphyResolver::resolve(raw)?
        } else if id == "liasmediaplayer:tenor" {
            TenorResolver::resolve(raw, cancel)?
        } else if kind == MediaKind::Image
            || [
                "liasmediaplayer:picture",
                "liasmediaplayer:video_file",
                "liasmediaplayer:audio_file",
                "liasmediaplayer:stream",
            ]
            .contains(&id)
        {
            DirectResolver::resolve(raw, kind)?
        } else {
            YtDlpResolver::resolve(&self.tools, raw, kind, options, cancel)?
        };
        self.cache.insert(key, media.clone(), SystemTime::now());
        Ok(media)
    }
    /// Playback should call this when a CDN rejects a previously resolved stream.
    pub fn on_http_status(&self, raw: &str, status: u16) {
        if status == 403 {
            for kind in [MediaKind::Image, MediaKind::Video, MediaKind::Audio] {
                self.cache.invalidate(&format!("{kind:?}:{raw}"));
            }
        }
    }
}

pub struct ExpandedCollection {
    pub title: Option<String>,
    pub entries: Vec<MediaRef>,
}
pub struct CollectionExpander;
impl CollectionExpander {
    pub const MAX_ENTRIES: usize = 500;
    pub fn expand(
        tools: &ToolManager,
        collection: &MediaRef,
        cancel: &Cancellation,
    ) -> io::Result<ExpandedCollection> {
        if collection.kind != MediaKind::Collection || web_url(&collection.url).is_none() {
            return Err(io::Error::other("Expected an HTTP(S) collection URL"));
        }
        let path = tools.require(Tool::YtDlp, cancel)?;
        let bytes = ProcessRunner::run(
            Command::new(path).args([
                "--ignore-config",
                "--flat-playlist",
                "--yes-playlist",
                "--ignore-errors",
                "--no-warnings",
                "--skip-download",
                "--playlist-end",
                "500",
                "-J",
                "--",
                &collection.url,
            ]),
            Duration::from_secs(120),
            32 * 1024 * 1024,
            cancel,
        )?;
        Self::parse(&bytes, collection)
    }
    pub fn parse(bytes: &[u8], collection: &MediaRef) -> io::Result<ExpandedCollection> {
        let root: Value = serde_json::from_slice(bytes).map_err(io::Error::other)?;
        let items = root["entries"]
            .as_array()
            .ok_or_else(|| io::Error::other("No playlist entries"))?;
        let sources = SourceRegistry::default();
        let mut entries = Vec::new();
        for item in items {
            if entries.len() == Self::MAX_ENTRIES {
                break;
            }
            if item.is_null()
                || matches!(
                    item["title"].as_str(),
                    Some("[Private video]" | "[Deleted video]")
                )
                || matches!(
                    item["availability"].as_str(),
                    Some("private" | "premium_only" | "subscriber_only")
                )
            {
                continue;
            }
            let url = ["webpage_url", "url"]
                .iter()
                .filter_map(|k| item[k].as_str())
                .find(|s| web_url(s).is_some())
                .map(str::to_owned)
                .or_else(|| {
                    let id = item["id"].as_str()?;
                    (item["ie_key"] == "Youtube"
                        && (6..=20).contains(&id.len())
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)))
                    .then(|| format!("https://www.youtube.com/watch?v={id}"))
                });
            let Some(url) = url else {
                continue;
            };
            let mut entry = sources
                .identify(&url)
                .filter(|r| matches!(r.kind, MediaKind::Audio | MediaKind::Video))
                .unwrap_or_else(|| MediaRef {
                    url,
                    kind: collection.entry_kind,
                    entry_kind: collection.entry_kind,
                    source_id: "liasmediaplayer:collection_entry".into(),
                    label: "collection_entry".into(),
                });
            // Flat entries may carry the parent list parameter; resolve each as one item.
            if let Some(mut url) = web_url(&entry.url)
                && matches!(
                    url.host_str(),
                    Some(
                        "www.youtube.com"
                            | "youtube.com"
                            | "m.youtube.com"
                            | "music.youtube.com"
                            | "youtu.be"
                    )
                )
            {
                let pairs: Vec<_> = url
                    .query_pairs()
                    .filter(|(k, _)| k != "list" && k != "index" && k != "start_radio")
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                url.set_query(None);
                if !pairs.is_empty() {
                    url.query_pairs_mut().extend_pairs(pairs);
                }
                entry.url = url.to_string();
            }
            if collection.entry_kind == MediaKind::Audio {
                entry.kind = MediaKind::Audio;
                entry.entry_kind = MediaKind::Audio;
            }
            entry.label = item["title"].as_str().unwrap_or(&entry.url).to_owned();
            entries.push(entry);
        }
        if entries.is_empty() {
            return Err(io::Error::other("No playable playlist entries"));
        }
        Ok(ExpandedCollection {
            title: root["title"].as_str().map(str::to_owned),
            entries,
        })
    }
}
