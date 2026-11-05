//! Library edits preserve Java entry shapes and unknown metadata.
use super::sources::{MediaKind, MediaRef, SourceRegistry};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

pub const LIMIT: usize = 500;
pub fn lists(library: &Value) -> &[Value] {
    library["playlists"].as_array().map_or(&[], Vec::as_slice)
}
pub fn entries(list: &Value) -> &[Value] {
    list[entry_key(list)].as_array().map_or(&[], Vec::as_slice)
}
fn entry_key(list: &Value) -> &'static str {
    ["entries", "items", "urls"]
        .into_iter()
        .find(|k| list[*k].is_array())
        .unwrap_or("entries")
}
pub fn name(list: &Value) -> &str {
    list["name"]
        .as_str()
        .or_else(|| list["title"].as_str())
        .unwrap_or("Playlist")
}
pub fn url(entry: &Value) -> Option<&str> {
    entry.as_str().or_else(|| entry["url"].as_str())
}
pub fn short_title(raw: &str) -> String {
    let Some(url) = super::sources::web_url(raw) else {
        return raw.chars().take(128).collect();
    };
    let host = url.host_str().unwrap_or("Media").trim_start_matches("www.");
    let part = url
        .path_segments()
        .and_then(|mut p| p.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or(host);
    if host.ends_with("youtube.com") {
        return url
            .query_pairs()
            .find(|(k, _)| k == "v")
            .map_or_else(|| "YouTube track".into(), |(_, v)| format!("YouTube · {v}"));
    }
    if host == "youtu.be" {
        return format!("YouTube · {part}");
    }
    part.replace("%20", " ").chars().take(128).collect()
}
pub fn label(entry: &Value) -> String {
    entry["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(256).collect())
        .unwrap_or_else(|| {
            url(entry)
                .map(short_title)
                .unwrap_or("Unavailable entry".into())
        })
}

/// Read clipboard M3U text or one HTTP(S) media link per line, preserving titles.
pub fn clipboard_entries(text: &str) -> Result<Vec<Value>, String> {
    let mut entries = Vec::new();
    let mut title = None;
    for line in text.lines().map(str::trim) {
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            title = info
                .split_once(',')
                .map(|(_, name)| name.trim().chars().take(256).collect::<String>());
        } else if super::sources::web_url(line).is_some() {
            let mut entry = json!({"url":line,"kind":"Audio"});
            if let Some(title) = title.take() {
                entry["title"] = title.into();
            }
            entries.push(entry);
            if entries.len() > LIMIT {
                return Err("Maximum 500 tracks per playlist".into());
            }
        }
    }
    if entries.is_empty() {
        return Err("Copy an M3U playlist or one media link per line first".into());
    }
    Ok(entries)
}
pub fn media(entry: &Value, sources: &SourceRegistry) -> Option<MediaRef> {
    let url = url(entry)?;
    sources.identify(url).or_else(|| {
        super::sources::web_url(url)?;
        let kind = match entry["kind"].as_str()?.to_ascii_lowercase().as_str() {
            "audio" => MediaKind::Audio,
            "video" => MediaKind::Video,
            "image" | "gif" => MediaKind::Image,
            _ => return None,
        };
        Some(MediaRef {
            url: url.into(),
            kind,
            entry_kind: kind,
            source_id: "liasmediaplayer:saved".into(),
            label: "saved".into(),
        })
    })
}
pub fn create(library: &mut Value, title: &str) -> Result<usize, String> {
    let title = valid_name(title)?;
    if !library["playlists"].is_array() {
        library["playlists"] = json!([]);
    }
    let lists = library["playlists"].as_array_mut().unwrap();
    if lists.len() >= LIMIT {
        return Err("Maximum 500 playlists".into());
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    lists.push(json!({"id":format!("rust-{stamp}-{}", NEXT.fetch_add(1,Ordering::Relaxed)),"name":title,"entries":[]}));
    Ok(lists.len() - 1)
}
fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
        return Err("Use a playlist name of 1–128 characters".into());
    }
    Ok(name.into())
}
pub fn rename(library: &mut Value, index: usize, title: &str) -> Result<(), String> {
    let title = valid_name(title)?;
    let list = library["playlists"]
        .get_mut(index)
        .filter(|v| v.is_object())
        .ok_or("Select a playlist")?;
    list["name"] = json!(title);
    Ok(())
}
pub fn delete(library: &mut Value, index: usize) -> Result<(), String> {
    let lists = library["playlists"]
        .as_array_mut()
        .ok_or("Select a playlist")?;
    if index >= lists.len() {
        return Err("Select a playlist".into());
    }
    lists.remove(index);
    Ok(())
}
pub fn append(library: &mut Value, index: usize, incoming: Vec<Value>) -> Result<(), String> {
    let list = library["playlists"]
        .get_mut(index)
        .filter(|v| v.is_object())
        .ok_or("Select a playlist")?;
    if entries(list).len() + incoming.len() > LIMIT {
        return Err("Maximum 500 entries per playlist; nothing added".into());
    }
    if incoming
        .iter()
        .any(|e| url(e).and_then(super::sources::web_url).is_none())
    {
        return Err("Expected HTTP(S) media URLs; nothing added".into());
    }
    let key = entry_key(list);
    if !list[key].is_array() {
        list[key] = json!([]);
    }
    list[key].as_array_mut().unwrap().extend(incoming);
    Ok(())
}
pub fn remove(library: &mut Value, index: usize, entry: usize) -> Result<(), String> {
    let list = library["playlists"]
        .get_mut(index)
        .filter(|v| v.is_object())
        .ok_or("Select a playlist")?;
    let key = entry_key(list);
    let entries = list[key].as_array_mut().ok_or("Select an entry")?;
    if entry >= entries.len() {
        return Err("Select an entry".into());
    }
    entries.remove(entry);
    Ok(())
}
pub fn reorder(library: &mut Value, index: usize, from: usize, to: usize) -> Result<(), String> {
    let list = library["playlists"]
        .get_mut(index)
        .filter(|v| v.is_object())
        .ok_or("Select a playlist")?;
    let key = entry_key(list);
    let entries = list[key].as_array_mut().ok_or("Select an entry")?;
    if from >= entries.len() || to >= entries.len() {
        return Err("Entry no longer exists".into());
    }
    let entry = entries.remove(from);
    entries.insert(to, entry);
    Ok(())
}
/// Indices keep history actions attached to the original entries after filtering.
pub fn history_indices(
    library: &Value,
    query: &str,
    source: Option<&str>,
    sources: &SourceRegistry,
) -> Vec<usize> {
    let query = query.to_lowercase();
    library["history"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, e)| {
            let matches =
                source.is_none_or(|id| media(e, sources).is_some_and(|m| m.source_id == id));
            matches
                && format!("{} {}", label(e), url(e).unwrap_or(""))
                    .to_lowercase()
                    .contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}
