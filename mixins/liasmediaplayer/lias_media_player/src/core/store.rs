//! Debounced snapshots and atomic replacement. Unknown JSON fields survive migrations.
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const DEBOUNCE: Duration = Duration::from_millis(500);
static TEMP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let id = TEMP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = parent.join(format!(
        ".{}.{}.{id}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
struct Pending {
    bytes: Option<Vec<u8>>,
    deadline: Instant,
    stop: bool,
    error: Option<String>,
}
pub struct JsonStore<T> {
    value: T,
    path: PathBuf,
    shared: Arc<(Mutex<Pending>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}
impl<T: Serialize + DeserializeOwned + PartialEq> JsonStore<T> {
    pub fn open(
        path: PathBuf,
        default: T,
        migrate: fn(Value) -> io::Result<Value>,
    ) -> io::Result<Self> {
        let mut migrated = false;
        let value = match fs::read(&path) {
            Ok(bytes) => {
                let decoded = (|| {
                    let raw: Value = serde_json::from_slice(&bytes)?;
                    let updated = migrate(raw.clone())?;
                    migrated = raw != updated;
                    serde_json::from_value(updated).map_err(io::Error::other)
                })();
                match decoded {
                    Ok(v) => v,
                    Err(e) => {
                        let suffix = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos();
                        fs::rename(&path, path.with_extension(format!("broken-{suffix}.json")))?;
                        log::warn!("Preserved unreadable {}: {e}", path.display());
                        migrated = true;
                        default
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                migrated = true;
                default
            }
            Err(e) => return Err(e),
        };
        let shared = Arc::new((
            Mutex::new(Pending {
                bytes: None,
                deadline: Instant::now(),
                stop: false,
                error: None,
            }),
            Condvar::new(),
        ));
        let state = shared.clone();
        let target = path.clone();
        let worker = thread::Builder::new()
            .name("lmp-json".into())
            .spawn(move || {
                let (lock, wake) = &*state;
                let mut p = lock.lock().unwrap();
                loop {
                    if p.stop {
                        break;
                    }
                    if p.bytes.is_none() {
                        p = wake.wait(p).unwrap();
                        continue;
                    }
                    let now = Instant::now();
                    if now < p.deadline {
                        let wait = p.deadline - now;
                        p = wake.wait_timeout(p, wait).unwrap().0;
                        continue;
                    }
                    match atomic_write(&target, p.bytes.as_ref().unwrap()) {
                        Ok(()) => {
                            p.bytes = None;
                            p.error = None;
                        }
                        Err(e) => {
                            p.error = Some(e.to_string());
                            p.deadline = Instant::now() + Duration::from_secs(2);
                        }
                    }
                }
            })?;
        let store = Self {
            value,
            path,
            shared,
            worker: Some(worker),
        };
        if migrated {
            store.schedule()?;
        }
        Ok(store)
    }
    pub fn get(&self) -> &T {
        &self.value
    }
    pub fn set(&mut self, value: T) -> io::Result<()> {
        if value != self.value {
            self.value = value;
            self.schedule()?;
        }
        Ok(())
    }
    fn schedule(&self) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.value)?;
        let (lock, wake) = &*self.shared;
        let mut p = lock.lock().unwrap();
        p.bytes = Some(bytes);
        p.deadline = Instant::now() + DEBOUNCE;
        wake.notify_one();
        Ok(())
    }
    pub fn flush(&self) -> io::Result<()> {
        flush(&self.shared, &self.path)
    }
    pub fn last_error(&self) -> Option<String> {
        self.shared.0.lock().unwrap().error.clone()
    }
}
fn flush(shared: &Arc<(Mutex<Pending>, Condvar)>, path: &Path) -> io::Result<()> {
    let mut p = shared.0.lock().unwrap();
    if let Some(bytes) = &p.bytes {
        atomic_write(path, bytes)?;
        p.bytes = None;
        p.error = None;
    }
    Ok(())
}
impl<T> Drop for JsonStore<T> {
    fn drop(&mut self) {
        if let Err(e) = flush(&self.shared, &self.path) {
            log::warn!("Could not flush {}: {e}", self.path.display());
        }
        self.shared.0.lock().unwrap().stop = true;
        self.shared.1.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
/// Accepts Java's `schema` and the original blueprint's `schemaVersion` envelope.
/// Data is additive: no unknown setting, playlist or window attribute is discarded.
pub fn migrate(mut value: Value) -> io::Result<Value> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| io::Error::other("expected a JSON object"))?;
    let version = object
        .get("schemaVersion")
        .or_else(|| object.get("schema"))
        .and_then(Value::as_u64)
        .unwrap_or(1);
    if version <= 3 {
        object.insert("schema".into(), json!(3));
        if object.contains_key("schemaVersion") {
            object.insert("schemaVersion".into(), json!(3));
        }
    }
    Ok(value)
}
pub struct Stores {
    pub settings: JsonStore<Value>,
    pub library: JsonStore<Value>,
    pub windows: JsonStore<Value>,
}
impl Stores {
    pub fn open(root: &Path, java: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        if !root.join("settings.json").exists() && root != java {
            for name in [
                "settings.json",
                "library.json",
                "windows.json",
                "config.json",
                "playlists.json",
                "history.json",
            ] {
                let source = java.join(name);
                let target = root.join(name);
                if source.is_file() && !target.exists() {
                    atomic_write(&target, &fs::read(source)?)?;
                }
            }
            // Copy only regular tool files; never follow directories or symlinks out of bin/.
            if let Ok(entries) = fs::read_dir(java.join("bin")) {
                fs::create_dir_all(root.join("bin"))?;
                for entry in entries.flatten() {
                    let target = root.join("bin").join(entry.file_name());
                    if entry.file_type()?.is_file() && !target.exists() {
                        fs::copy(entry.path(), &target)?;
                    }
                }
            }
        }
        import_v2(root)?;
        Ok(Self {
            settings: JsonStore::open(
                root.join("settings.json"),
                json!({"schema":3,"theme":"dark","replaceChatLinks":true,"memoryCacheSizeMb":128,"diskCacheSizeMb":100}),
                migrate,
            )?,
            library: JsonStore::open(
                root.join("library.json"),
                json!({"schema":3,"playlists":[],"history":[],"favourites":[]}),
                migrate,
            )?,
            windows: JsonStore::open(
                root.join("windows.json"),
                json!({"schema":3,"kinds":{},"pinned":[]}),
                migrate,
            )?,
        })
    }
    pub fn record_image(&mut self, url: &str) -> io::Result<()> {
        self.record_media(url, "IMAGE", "")
    }
    pub fn record_media(&mut self, url: &str, kind: &str, title: &str) -> io::Result<()> {
        let mut lib = self.library.get().clone();
        if !lib["history"].is_array() {
            lib["history"] = json!([]);
        }
        let h = lib["history"].as_array_mut().unwrap();
        let mut entry = h
            .iter()
            .find(|e| e["url"] == url)
            .cloned()
            .unwrap_or(json!({"favorite":false}));
        entry["url"] = json!(url);
        entry["kind"] = json!(kind.to_ascii_uppercase());
        if !title.is_empty() {
            entry["title"] = json!(title);
        }
        entry["lastPlayed"] = json!(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
        );
        h.retain(|e| e["url"] != url);
        h.insert(0, entry);
        h.truncate(500);
        self.library.set(lib)
    }
    pub fn flush(&self) -> io::Result<()> {
        self.settings.flush()?;
        self.library.flush()?;
        self.windows.flush()
    }
}
fn read_optional(path: &Path) -> io::Result<Option<Value>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(serde_json::from_slice(&b)?)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn import_v2(root: &Path) -> io::Result<()> {
    if root.join("settings.json").exists() {
        return Ok(());
    }
    let config = read_optional(&root.join("config.json"))?;
    let playlists = read_optional(&root.join("playlists.json"))?;
    let history = read_optional(&root.join("history.json"))?;
    if config.is_none() && playlists.is_none() && history.is_none() {
        return Ok(());
    }
    let mut settings = config.unwrap_or(json!({}));
    if !settings.is_object() {
        return Err(io::Error::other("v2 config must be an object"));
    }
    let mut values = settings
        .get("values")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or(json!({}));
    for (key, value) in settings.as_object().unwrap().clone() {
        let Some(name) = key.strip_prefix("liasmediaplayer:") else {
            if key != "values" {
                values[&key] = value;
            }
            continue;
        };
        let mapped = match name {
            "max_video_windows" => "windows.max_video",
            "max_audio_windows" => "windows.max_audio",
            "max_pinned_images" => "windows.max_images",
            "max_image_cache_mb" => "performance.image_cache_mb",
            "default_window_position" => "windows.default_position",
            "link_filter_mode" => "chat.link_filter",
            "auto_update_tools" => "tools.auto_update",
            "blocked_domains" => "chat.blocked_hosts",
            "allowed_domains" => "chat.allowed_hosts",
            "blocked_senders" => "chat.blocked_senders",
            "video_resolution" => "video.max_height",
            "theme" => "appearance.theme",
            _ => name,
        };
        let converted = match name {
            "blocked_domains" | "allowed_domains" | "blocked_senders" => value
                .as_str()
                .map(|s| {
                    json!(
                        s.split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                    )
                })
                .unwrap_or(value),
            "video_resolution" => json!(
                ["P144", "P240", "P360", "P480", "P720"]
                    [value.as_i64().unwrap_or(4).clamp(0, 4) as usize]
            ),
            "theme" => value
                .as_str()
                .map(|s| {
                    if s.contains(':') {
                        json!(s.trim())
                    } else {
                        json!(format!("liasmediaplayer:{}", s.trim()))
                    }
                })
                .unwrap_or(value),
            _ => value,
        };
        values[format!("liasmediaplayer:{mapped}")] = converted;
    }
    settings["values"] = values;
    settings["schema"] = json!(3);
    for (old, new) in [
        ("max_image_cache_mb", "memoryCacheSizeMb"),
        ("disk_cache_mb", "diskCacheSizeMb"),
        ("replace_chat_links", "replaceChatLinks"),
    ] {
        if let Some(v) = settings.get(old).cloned() {
            settings[new] = v;
        }
    }
    if !root.join("library.json").exists() {
        let mut h = history.unwrap_or(json!([]));
        if let Some(entries) = h.as_array_mut() {
            entries.retain(|e| {
                e["url"]
                    .as_str()
                    .is_some_and(|s| super::sources::web_url(s).is_some())
            });
            for entry in entries.iter_mut() {
                if let Some(played) = entry.get("playedAt").cloned() {
                    entry["lastPlayed"] = played;
                }
            }
            entries.truncate(500);
        }
        let mut playlists = playlists.unwrap_or(json!([]));
        if let Some(entries) = playlists.as_array_mut() {
            entries.retain(Value::is_object);
            for (i, entry) in entries.iter_mut().enumerate() {
                if entry
                    .get("id")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    entry["id"] = json!(format!("imported-{i}"));
                }
            }
        }
        atomic_write(
            &root.join("library.json"),
            &serde_json::to_vec_pretty(&json!({"schema":3,"playlists":playlists,"history":h}))?,
        )?;
    }
    if let Some(mut w) = read_optional(&root.join("windows.json"))?
        && w.get("kinds").is_none()
    {
        if let Some(o) = w.as_object_mut() {
            o.remove("schema");
            o.remove("schemaVersion");
        }
        if let Some(kinds) = w.as_object_mut() {
            for state in kinds.values_mut().filter(|v| v.is_object()) {
                if let Some(queue) = state.get("queuePanel").cloned() {
                    state["queueOpen"] = queue;
                }
                if state["sized"].as_bool() == Some(true)
                    && let Some(width) = state["width"].as_u64()
                {
                    state["width"] = json!(width.saturating_add(2));
                }
            }
        }
        let path = root.join("windows.json");
        atomic_write(&root.join("windows.v2.bak.json"), &fs::read(&path)?)?;
        atomic_write(
            &path,
            &serde_json::to_vec_pretty(&json!({"schema":3,"kinds":w,"pinned":[]}))?,
        )?;
    }
    // Originals remain untouched; settings.json is the completion marker written last.
    atomic_write(
        &root.join("settings.json"),
        &serde_json::to_vec_pretty(&settings)?,
    )
}
