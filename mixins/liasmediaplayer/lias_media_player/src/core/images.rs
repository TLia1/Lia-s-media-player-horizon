//! One bounded image worker; stale completions cannot resurrect closed/replaced windows.
use super::{
    cache::{AsyncCache, DiskCache},
    image::{DecodedImage, ImageLoader},
    process::Cancellation,
    resolver::{ResolveOptions, ResolverChain},
    sources::{MediaKind, SourceRegistry},
    store::Stores,
    tools::ToolManager,
};
use crate::{
    platform::{Rect, TextureId, horizon_impl::ModAtlasPacker},
    ui::{
        FloatingWindow, WindowManager,
        image_window::{ImageView, ImageWindow},
    },
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};
struct Request {
    id: u64,
    generation: u64,
    url: String,
    cancel: Cancellation,
}
struct Completion {
    request: Request,
    result: Result<Arc<DecodedImage>, String>,
}
struct Entry {
    url: String,
    generation: u64,
    view: Arc<Mutex<ImageView>>,
    texture: Option<TextureId>,
    cancel: Cancellation,
}
pub struct Images {
    tx: SyncSender<Request>,
    rx: Receiver<Completion>,
    entries: HashMap<u64, Entry>,
    generation: u64,
    cancel: Cancellation,
    pub sources: SourceRegistry,
    tools: Arc<Mutex<ToolManager>>,
}
impl Images {
    pub fn new(root: Option<&Path>, memory: usize, disk: u64) -> Self {
        Self::with_tools(root, memory, disk, ToolManager::new(root))
    }
    pub fn with_tools(root: Option<&Path>, memory: usize, disk: u64, tools: ToolManager) -> Self {
        let cancel = Cancellation::default();
        let worker_cancel = cancel.clone();
        let (tx, jobs) = mpsc::sync_channel::<Request>(8);
        let (done, rx) = mpsc::sync_channel(8);
        let root = root.map(Path::to_path_buf);
        let tools = Arc::new(Mutex::new(tools));
        let worker_tools = tools.clone();
        std::thread::Builder::new()
            .name("lmp-images".into())
            .spawn(move || {
                let mut resolver = ResolverChain::new(worker_tools.lock().clone());
                let cache = AsyncCache::new(memory);
                let disk = root.and_then(|p| match DiskCache::open(&p.join("cache"), disk) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        log::warn!("Image disk cache: {e}");
                        None
                    }
                });
                while let Ok(request) = jobs.recv() {
                    if worker_cancel.is_cancelled() {
                        break;
                    }
                    if request.cancel.is_cancelled() {
                        continue;
                    }
                    resolver.tools = worker_tools.lock().clone();
                    let url = request.url.clone();
                    let result = cache.get_or_load(url.clone(), || {
                        let cached = disk.as_ref().and_then(|d| d.get(&url).ok().flatten());
                        if let Some(bytes) = cached {
                            match ImageLoader::decode_with_tools(
                                &bytes,
                                &resolver.tools,
                                &request.cancel,
                            ) {
                                Ok(image) => {
                                    let image = image.fit(512);
                                    let weight = image.weight();
                                    return Ok((image, weight));
                                }
                                Err(_) => {
                                    if let Some(d) = &disk {
                                        d.remove(&url);
                                    }
                                }
                            }
                        }
                        let media = resolver
                            .resolve(
                                &url,
                                MediaKind::Image,
                                ResolveOptions::default(),
                                &request.cancel,
                            )
                            .map_err(|e| e.to_string())?;
                        let direct = media.video.as_ref().ok_or("Resolver returned no image")?;
                        let bytes =
                            ImageLoader::download(&direct.url).map_err(|e| e.to_string())?;
                        let image = ImageLoader::decode_with_tools(
                            &bytes,
                            &resolver.tools,
                            &request.cancel,
                        )
                        .map_err(|e| e.to_string())?
                        .fit(512);
                        if let Some(d) = &disk
                            && let Err(e) = d.put(&url, &bytes)
                        {
                            log::warn!("Image cache write: {e}");
                        }
                        let weight = image.weight();
                        Ok((image, weight))
                    });
                    if done.send(Completion { request, result }).is_err() {
                        break;
                    }
                }
            })
            .expect("start image worker");
        Self {
            tx,
            rx,
            entries: HashMap::new(),
            generation: 0,
            cancel,
            sources: SourceRegistry::default(),
            tools,
        }
    }
    pub fn set_tools(&self, tools: ToolManager) {
        *self.tools.lock() = tools;
    }
    pub fn request(
        &mut self,
        url: &str,
        new_window: bool,
        windows: &mut WindowManager,
        atlas: &mut ModAtlasPacker,
        geometry: Option<&Value>,
    ) -> Result<u64, String> {
        self.entries.retain(|id, entry| {
            if windows.windows().iter().any(|w| w.id == *id) {
                true
            } else {
                if let Some(t) = entry.texture {
                    atlas.free(t);
                }
                false
            }
        });
        if let Some(source) = self.sources.identify(url)
            && source.kind != MediaKind::Image
        {
            return Err("This link is not an image".into());
        }
        super::sources::web_url(url).ok_or("Expected an HTTP(S) image URL")?;
        let reuse = (!new_window)
            .then(|| {
                windows
                    .windows()
                    .iter()
                    .rev()
                    .find(|w| self.entries.contains_key(&w.id))
                    .map(|w| w.id)
            })
            .flatten();
        if reuse.is_none() && self.entries.len() >= 4 {
            return Err("Close an image window first (maximum four)".into());
        }
        let view = Arc::new(Mutex::new(ImageView::default()));
        let mut window = FloatingWindow::new(
            "Image",
            Rect::new(30., 35., 320., 240.),
            Box::new(ImageWindow::new(view.clone())),
        );
        if let Some(g) = geometry {
            let finite = |key: &str, default: f32| {
                g[key]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .map(|v| v as f32)
                    .filter(|v| v.is_finite())
                    .unwrap_or(default)
            };
            window.rect = Rect::new(
                finite("x", 30.),
                finite("y", 35.),
                finite("width", 320.).max(120.),
                finite("height", 240.).max(80.),
            );
            window.pinned = g["pinned"].as_bool().unwrap_or(true);
        }
        if let Some(id) = reuse {
            window.id = id;
            if let Some(old) = windows.windows().iter().find(|w| w.id == id) {
                window.rect = old.rect;
                window.pinned = old.pinned;
            }
        }
        let id = window.id;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let cancel = Cancellation::default();
        self.tx
            .try_send(Request {
                id,
                generation,
                url: url.into(),
                cancel: cancel.clone(),
            })
            .map_err(|_| "Image loader is busy; try again shortly")?;
        if let Some(old) = self.entries.remove(&id)
            && let Some(texture) = old.texture
        {
            atlas.free(texture);
        }
        windows.close(id);
        windows.open(window);
        self.entries.insert(
            id,
            Entry {
                url: url.into(),
                generation,
                view,
                texture: None,
                cancel,
            },
        );
        Ok(id)
    }
    pub fn tick(
        &mut self,
        dt: Duration,
        windows: &WindowManager,
        atlas: &mut ModAtlasPacker,
        mut stores: Option<&mut Stores>,
    ) {
        self.entries.retain(|id, e| {
            if windows.windows().iter().any(|w| w.id == *id) {
                true
            } else {
                if let Some(t) = e.texture {
                    atlas.free(t);
                }
                false
            }
        });
        while let Ok(completion) = self.rx.try_recv() {
            let Some(entry) = self
                .entries
                .get_mut(&completion.request.id)
                .filter(|e| e.generation == completion.request.generation)
            else {
                continue;
            };
            let mut view = entry.view.lock();
            match completion.result {
                Ok(image) => match atlas.allocate(image.width, image.height, &image.frames[0].rgba)
                {
                    Ok(texture) => {
                        entry.texture = Some(texture);
                        view.texture = Some(texture);
                        view.image = Some(image);
                        view.status.clear();
                        if let Some(s) = stores.as_deref_mut()
                            && let Err(e) = s.record_image(&entry.url)
                        {
                            log::warn!("Image history: {e}");
                        }
                    }
                    Err(e) => {
                        view.status = format!("Atlas full: {e:?}. Close an image and try again.")
                    }
                },
                Err(e) => view.status = e,
            }
        }
        for e in self.entries.values() {
            let mut view = e.view.lock();
            view.advance(dt);
            if view.dirty {
                if let (Some(t), Some(image)) = (view.texture, &view.image)
                    && let Err(e) = atlas.update(t, &image.frames[view.frame].rgba)
                {
                    log::warn!("Image upload: {e:?}");
                }
                view.dirty = false;
            }
        }
    }
    pub fn persist(&self, windows: &WindowManager, stores: &mut Stores) {
        let mut value = stores.windows.get().clone();
        let mut pinned = Vec::new();
        for w in windows.windows() {
            let Some(e) = self.entries.get(&w.id) else {
                continue;
            };
            let mut g = value["kinds"]["image"]
                .as_object()
                .cloned()
                .map(Value::Object)
                .unwrap_or(json!({}));
            for (k, v) in [
                ("placed", json!(true)),
                ("x", json!(w.rect.x)),
                ("y", json!(w.rect.y)),
                ("width", json!(w.rect.w)),
                ("height", json!(w.rect.h)),
                ("pinned", json!(w.pinned)),
            ] {
                g[k] = v;
            }
            if !value["kinds"].is_object() {
                value["kinds"] = json!({});
            }
            value["kinds"]["image"] = g.clone();
            if w.pinned {
                g["url"] = json!(e.url);
                pinned.push(g);
            }
        }
        value["pinned"] = json!(pinned);
        if let Err(e) = stores.windows.set(value) {
            log::warn!("Window persistence: {e}");
        }
    }
    pub fn clear(&mut self, atlas: &mut ModAtlasPacker) {
        for (_, e) in self.entries.drain() {
            if let Some(t) = e.texture {
                atlas.free(t);
            }
        }
    }
    pub fn source_url(&self, id: u64) -> Option<&str> {
        self.entries.get(&id).map(|e| e.url.as_str())
    }
    pub fn view(&self, id: u64) -> Option<Arc<Mutex<ImageView>>> {
        self.entries.get(&id).map(|e| e.view.clone())
    }
}

impl Drop for Images {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
