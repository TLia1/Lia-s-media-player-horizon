//! Bounded, cancellable hover work. No network, disk, or subprocess work on the UI thread.
use super::{
    cache::{AsyncCache, DiskCache},
    image::{DecodedImage, ImageLoader},
    process::Cancellation,
    resolver::{CollectionExpander, ExpandedCollection, ResolveOptions, ResolverChain},
    sources::{MediaKind, MediaRef},
    tools::ToolManager,
};
use std::{
    path::Path,
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

pub struct Preview {
    pub title: String,
    pub image: Option<DecodedImage>,
    pub detail: String,
    pub error: Option<String>,
    created: Instant,
}
impl Preview {
    fn weight(&self) -> usize {
        1024 + self.title.len() + self.image.as_ref().map_or(0, DecodedImage::weight)
    }
    fn fresh(&self) -> bool {
        self.created.elapsed()
            < if self.error.is_some() {
                Duration::from_secs(60)
            } else {
                Duration::from_secs(3600)
            }
    }
}
struct Request {
    media: MediaRef,
    cancel: Cancellation,
}
pub struct PreviewCache {
    tx: SyncSender<Request>,
    cache: Arc<AsyncCache<String, Preview>>,
    active: Option<(String, Cancellation)>,
    tools: Arc<parking_lot::Mutex<ToolManager>>,
}
impl PreviewCache {
    pub fn new(root: Option<&Path>, mut tools: ToolManager) -> Self {
        // Merely hovering must not install tools. Explicit playback retains the user's policy.
        tools.policy.auto_download = false;
        let root = root.map(|r| r.join("chat-cache"));
        let cache = Arc::new(AsyncCache::new(8 * 1024 * 1024));
        let worker_cache = cache.clone();
        let (tx, rx) = mpsc::sync_channel::<Request>(2);
        let tools = Arc::new(parking_lot::Mutex::new(tools));
        let worker_tools = tools.clone();
        std::thread::Builder::new()
            .name("lmp-chat-preview".into())
            .spawn(move || {
                let mut resolver = ResolverChain::new(worker_tools.lock().clone());
                let mut disk = root.and_then(|r| DiskCache::open(&r, 32 * 1024 * 1024).ok());
                while let Ok(job) = rx.recv() {
                    if job.cancel.is_cancelled() {
                        continue;
                    }
                    resolver.tools = worker_tools.lock().clone();
                    let mut preview = Preview {
                        title: disk
                            .as_ref()
                            .and_then(|d| d.title(&job.media.url))
                            .unwrap_or(&job.media.label)
                            .into(),
                        image: None,
                        detail: String::new(),
                        error: None,
                        created: Instant::now(),
                    };
                    let result = load(&resolver, &job, disk.as_mut(), &mut preview);
                    if job.cancel.is_cancelled() {
                        continue;
                    }
                    preview.error = result.err().map(|e| e.to_string());
                    let weight = preview.weight();
                    worker_cache.insert(job.media.url, Arc::new(preview), weight);
                }
            })
            .expect("chat preview worker");
        Self {
            tx,
            cache,
            active: None,
            tools,
        }
    }
    pub fn set_tools(&self, mut tools: ToolManager) {
        tools.policy.auto_download = false;
        *self.tools.lock() = tools;
    }
    pub fn get(&self, url: &str) -> Option<Arc<Preview>> {
        self.cache.get(&url.to_owned()).filter(|p| p.fresh())
    }
    pub fn request(&mut self, media: &MediaRef) {
        if self
            .active
            .as_ref()
            .is_some_and(|(url, _)| *url == media.url)
        {
            return;
        }
        self.cancel();
        if self.get(&media.url).is_some() {
            return;
        }
        let cancel = Cancellation::default();
        if self
            .tx
            .try_send(Request {
                media: media.clone(),
                cancel: cancel.clone(),
            })
            .is_ok()
        {
            self.active = Some((media.url.clone(), cancel));
        }
    }
    pub fn cancel(&mut self) {
        if let Some((_, cancel)) = self.active.take() {
            cancel.cancel();
        }
    }
}
impl Drop for PreviewCache {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn load(
    resolver: &ResolverChain,
    job: &Request,
    mut disk: Option<&mut DiskCache>,
    preview: &mut Preview,
) -> Result<(), Box<dyn std::error::Error>> {
    if job.media.kind == MediaKind::Collection {
        preview.detail = "Collection · click to browse tracks".into();
        return Ok(());
    }
    // Restore encoded thumbnails without resolving provider URLs again.
    if let Some(bytes) = disk
        .as_ref()
        .and_then(|d| d.get(&job.media.url).ok().flatten())
    {
        match ImageLoader::decode_with_tools(&bytes, &resolver.tools, &job.cancel) {
            Ok(mut image) => {
                image.frames.truncate(1);
                preview.image = Some(image.fit(192));
                return Ok(());
            }
            Err(_) => {
                if let Some(d) = &disk {
                    d.remove(&job.media.url);
                }
            }
        }
    }
    let media = resolver.resolve(
        &job.media.url,
        job.media.kind,
        ResolveOptions {
            max_height: 360,
            audio_only: job.media.kind == MediaKind::Audio,
        },
        &job.cancel,
    )?;
    if let Some(title) = media.title {
        preview.title = title.chars().take(256).collect();
    }
    preview.detail = if media.is_live {
        "LIVE".into()
    } else {
        media
            .duration
            .map(|d| format!("{}:{:02}", d as u64 / 60, d as u64 % 60))
            .unwrap_or_default()
    };
    if let Some(d) = disk.as_mut() {
        let _ = d.set_title(&job.media.url, &preview.title);
    }
    let thumb = if job.media.kind == MediaKind::Image {
        media.video.map(|s| s.url)
    } else {
        media.thumbnail
    };
    if let Some(url) = thumb {
        if job.cancel.is_cancelled() {
            return Ok(());
        }
        let bytes = ImageLoader::download(&url)?;
        if job.cancel.is_cancelled() {
            return Ok(());
        }
        let mut image = ImageLoader::decode_with_tools(&bytes, &resolver.tools, &job.cancel)?;
        image.frames.truncate(1);
        preview.image = Some(image.fit(192));
        if let Some(d) = disk {
            let _ = d.put(&job.media.url, &bytes);
        }
    }
    Ok(())
}
/// User-initiated collection expansion is separate from passive hover jobs.
pub struct CollectionTask {
    rx: Receiver<Result<ExpandedCollection, String>>,
    cancel: Cancellation,
    pub enqueue: bool,
}
impl CollectionTask {
    pub fn start(tools: ToolManager, media: MediaRef, enqueue: bool) -> Self {
        let (tx, rx) = mpsc::sync_channel(1);
        let cancel = Cancellation::default();
        let worker_cancel = cancel.clone();
        std::thread::spawn(move || {
            let result = CollectionExpander::expand(&tools, &media, &worker_cancel)
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        Self {
            rx,
            cancel,
            enqueue,
        }
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
    pub fn poll(&self) -> Option<Result<ExpandedCollection, String>> {
        self.rx.try_recv().ok()
    }
}
impl Drop for CollectionTask {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
