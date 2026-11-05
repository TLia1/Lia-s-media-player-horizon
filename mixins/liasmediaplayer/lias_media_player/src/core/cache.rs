use super::store::{JsonStore, atomic_write};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    hash::Hash,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
};

struct Entry<V> {
    value: Arc<V>,
    weight: usize,
    used: u64,
}
struct State<K, V> {
    entries: HashMap<K, Entry<V>>,
    loading: std::collections::HashSet<K>,
    weight: usize,
    clock: u64,
}
/// Weighted LRU with single-flight loading. Loaders run outside the lock, on their caller's
/// worker thread; concurrent requests for the same key share the completed value.
pub struct AsyncCache<K, V> {
    budget: usize,
    state: Mutex<State<K, V>>,
    ready: Condvar,
}
impl<K: Eq + Hash + Clone, V> AsyncCache<K, V> {
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            state: Mutex::new(State {
                entries: HashMap::new(),
                loading: Default::default(),
                weight: 0,
                clock: 0,
            }),
            ready: Condvar::new(),
        }
    }
    pub fn weight(&self) -> usize {
        self.state.lock().unwrap().weight
    }
    pub fn get(&self, key: &K) -> Option<Arc<V>> {
        let mut s = self.state.lock().unwrap();
        s.clock += 1;
        let now = s.clock;
        s.entries.get_mut(key).map(|e| {
            e.used = now;
            e.value.clone()
        })
    }
    pub fn insert(&self, key: K, value: Arc<V>, weight: usize) {
        let mut s = self.state.lock().unwrap();
        if let Some(old) = s.entries.remove(&key) {
            s.weight -= old.weight;
        }
        if weight > self.budget {
            return;
        }
        while s.weight > self.budget - weight {
            let Some(key) = s
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            s.weight -= s.entries.remove(&key).unwrap().weight;
        }
        s.clock += 1;
        let used = s.clock;
        s.weight += weight;
        s.entries.insert(
            key,
            Entry {
                value,
                weight,
                used,
            },
        );
    }
    pub fn get_or_load<E>(
        &self,
        key: K,
        load: impl FnOnce() -> Result<(V, usize), E>,
    ) -> Result<Arc<V>, E> {
        {
            let mut s = self.state.lock().unwrap();
            loop {
                s.clock += 1;
                let now = s.clock;
                if let Some(e) = s.entries.get_mut(&key) {
                    e.used = now;
                    return Ok(e.value.clone());
                }
                if s.loading.insert(key.clone()) {
                    break;
                }
                s = self.ready.wait(s).unwrap();
            }
        }
        // Always wake waiters, including when an addon loader unwinds.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(load));
        let result = result.map(|r| {
            r.map(|(value, weight)| {
                let value = Arc::new(value);
                self.insert(key.clone(), value.clone(), weight);
                value
            })
        });
        self.state.lock().unwrap().loading.remove(&key);
        self.ready.notify_all();
        match result {
            Ok(r) => r,
            Err(p) => std::panic::resume_unwind(p),
        }
    }
}
/// SHA-256 names prevent URL path traversal. Only owned `.thumb` files are evicted.
pub struct DiskCache {
    root: PathBuf,
    budget: u64,
    titles: JsonStore<serde_json::Value>,
}
impl DiskCache {
    pub fn open(root: &Path, budget: u64) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        Ok(Self {
            root: root.into(),
            budget,
            titles: JsonStore::open(root.join("titles.json"), serde_json::json!({}), |v| {
                if v.is_object() {
                    Ok(v)
                } else {
                    Err(io::Error::other("titles must be an object"))
                }
            })?,
        })
    }
    pub fn key(url: &str) -> String {
        hex::encode(Sha256::digest(url.as_bytes()))
    }
    fn path(&self, url: &str) -> PathBuf {
        self.root.join(format!("{}.thumb", Self::key(url)))
    }
    pub fn get(&self, url: &str) -> io::Result<Option<Vec<u8>>> {
        let path = self.path(url);
        match fs::File::open(&path) {
            Ok(mut file) => {
                use std::io::Read;
                if file.metadata()?.len() > self.budget.min(super::image::MAX_DOWNLOAD as u64) {
                    return Ok(None);
                }
                let mut b = Vec::new();
                (&mut file)
                    .take(super::image::MAX_DOWNLOAD as u64 + 1)
                    .read_to_end(&mut b)?;
                if b.len() > super::image::MAX_DOWNLOAD {
                    return Ok(None);
                }
                let _ = file.set_modified(std::time::SystemTime::now());
                Ok(Some(b))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn put(&self, url: &str, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() as u64 > self.budget || bytes.len() > super::image::MAX_DOWNLOAD {
            return Ok(());
        }
        atomic_write(&self.path(url), bytes)?;
        let mut files = Vec::new();
        let mut total = 0;
        for e in fs::read_dir(&self.root)? {
            let e = e?;
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.len() != 70
                || !name.ends_with(".thumb")
                || !name.as_bytes()[..64].iter().all(u8::is_ascii_hexdigit)
            {
                continue;
            }
            let meta = e.metadata()?;
            total += meta.len();
            files.push((meta.modified()?, meta.len(), e.path()));
        }
        files.sort_by_key(|f| f.0);
        for (_, len, path) in files {
            if total <= self.budget {
                break;
            }
            fs::remove_file(path)?;
            total -= len;
        }
        Ok(())
    }
    pub fn remove(&self, url: &str) {
        let _ = fs::remove_file(self.path(url));
    }
    pub fn title(&self, url: &str) -> Option<&str> {
        self.titles.get().get(Self::key(url))?.as_str()
    }
    pub fn set_title(&mut self, url: &str, title: &str) -> io::Result<()> {
        let mut v = self.titles.get().clone();
        v[Self::key(url)] = title.into();
        self.titles.set(v)
    }
}
