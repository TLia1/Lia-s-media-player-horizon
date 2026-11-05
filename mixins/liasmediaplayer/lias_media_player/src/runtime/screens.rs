use super::Runtime;
use crate::{
    core::{
        library,
        player::PlayerCommand,
        previews::CollectionTask,
        sources::{MediaKind, MediaRef},
        tools::{Tool, ToolTask},
    },
    ui::screens::{ScreenAction, ScreenKind},
};
use serde_json::{Value, json};

impl Runtime {
    pub fn open_screen(&mut self, kind: ScreenKind) {
        self.chat
            .clear_hover(self.previews.as_mut(), &mut self.ui.atlas);
        self.chat.picker = None;
        self.ui.windows.cancel_input();
        self.screens.open(kind);
        self.sync_screens();
    }
    /// Called when leaving our own screen, not when navigating between its views.
    pub fn close_views(&mut self) {
        self.screens.close();
        if let Some((_, task)) = &self.import_task {
            task.cancel();
        }
        self.screens.playlists.importing = false;
        // Keep cancelled jobs admitted until completion; rapid reopen cannot grow worker count.
        if let Some(task) = &self.tool_task {
            task.cancel();
        }
    }
    pub fn sync_screens(&mut self) {
        if self.screens.active.is_none() {
            return;
        }
        let empty = json!({});
        let library = self.stores.as_ref().map_or(&empty, |s| s.library.get());
        self.screens.sync(
            library,
            &self.sources,
            &self.ui.windows,
            self.playback.as_ref(),
        );
        if let Some(stores) = &self.stores
            && let Some(e) = stores
                .library
                .last_error()
                .or_else(|| stores.settings.last_error())
        {
            self.screens.notice = format!("Save pending; retrying: {e}");
        }
    }
    pub fn screen_action(&mut self, action: ScreenAction) {
        match self.apply_screen_action(action) {
            Ok(()) => {}
            Err(e) => self.screens.notice = e,
        }
        self.sync_screens();
    }
    fn edit_library(
        &mut self,
        edit: impl FnOnce(&mut Value) -> Result<(), String>,
    ) -> Result<(), String> {
        let stores = self
            .stores
            .as_mut()
            .ok_or("Library storage is unavailable")?;
        let mut value = stores.library.get().clone();
        edit(&mut value)?;
        stores.library.set(value).map_err(|e| e.to_string())
    }
    fn apply_screen_action(&mut self, action: ScreenAction) -> Result<(), String> {
        use ScreenAction::*;
        match action {
            Navigate(kind) => {
                self.open_screen(kind);
            }
            Close => {
                self.close_views();
                self.close_screen_requested = true;
            }
            Create => {
                let name = self.screens.playlists.new_name.text().trim().to_owned();
                let name = if name.is_empty() {
                    "New playlist".to_owned()
                } else {
                    name
                };
                let mut index = 0;
                self.edit_library(|v| {
                    index = library::create(v, &name)?;
                    Ok(())
                })?;
                self.screens.select_playlist(index);
                self.screens.playlists.new_name = crate::ui::TextField::new("");
                self.screens.playlists.new_name.placeholder = "New playlist".into();
                self.screens.notice = "Playlist created. Add links on the right.".into();
            }
            Rename => {
                let i = self
                    .screens
                    .selected_playlist()
                    .ok_or("Select a playlist")?;
                let name = self.screens.playlists.name.text().to_owned();
                self.edit_library(|v| library::rename(v, i, &name))?;
                self.screens.notice = "Playlist renamed".into();
            }
            DeletePlaylist => {
                let i = self
                    .screens
                    .selected_playlist()
                    .ok_or("Select a playlist")?;
                self.edit_library(|v| library::delete(v, i))?;
                self.screens.playlists.names.selected = None;
                self.screens.playlists.tracks.reset_scroll();
                self.screens.notice = "Playlist deleted".into();
            }
            AddUrl => {
                let i = self
                    .screens
                    .selected_playlist()
                    .ok_or("Select a playlist")?;
                let url = self.screens.playlists.url.text().trim().to_owned();
                let media = self
                    .sources
                    .identify(&url)
                    .ok_or("Use a supported media URL")?;
                if media.kind == MediaKind::Collection {
                    return self.start_import();
                }
                self.edit_library(|v| {
                    library::append(
                        v,
                        i,
                        vec![json!({"url":url,"kind":format!("{:?}",media.kind)})],
                    )
                })?;
                self.screens.playlists.url = crate::ui::TextField::new("");
                self.screens.playlists.url.placeholder = "Paste media or playlist link".into();
                self.screens.notice = "Track added".into();
            }
            Import => self.start_import()?,
            CancelImport => {
                if let Some((_, task)) = &self.import_task {
                    task.cancel();
                }
                self.screens.playlists.importing = false;
                self.screens.notice = "Import cancelled".into();
            }
            RemoveEntry => {
                let i = self
                    .screens
                    .selected_playlist()
                    .ok_or("Select a playlist")?;
                let e = self.screens.selected_entry().ok_or("Select an entry")?;
                self.edit_library(|v| library::remove(v, i, e))?;
                self.screens.playlists.tracks.selected = None;
            }
            Reorder(from, to) => {
                let i = self
                    .screens
                    .selected_playlist()
                    .ok_or("Select a playlist")?;
                self.edit_library(|v| library::reorder(v, i, from, to))?;
                self.screens.notice = "Playlist order saved".into();
            }
            PlayEntry(queue) => {
                let i = self
                    .screens
                    .selected_playlist()
                    .ok_or("Select a playlist")?;
                let e = self.screens.selected_entry().ok_or("Select an entry")?;
                let entry = self
                    .stores
                    .as_ref()
                    .and_then(|s| library::lists(s.library.get()).get(i))
                    .and_then(|p| library::entries(p).get(e))
                    .ok_or("Entry no longer exists")?;
                let mut media =
                    library::media(entry, &self.sources).ok_or("Unsupported media URL")?;
                if matches!(media.kind, MediaKind::Audio | MediaKind::Video) {
                    media.kind = MediaKind::Audio;
                    media.entry_kind = MediaKind::Audio;
                }
                self.open_chat_media(media, queue);
                self.screens.notice = "Playback requested".into();
            }
            PlayPlaylist(_) | QueuePlaylist => {
                let shuffle = matches!(action, PlayPlaylist(true));
                let i = self
                    .screens
                    .selected_playlist()
                    .ok_or("Select a playlist")?;
                let playlist = self
                    .stores
                    .as_ref()
                    .and_then(|s| library::lists(s.library.get()).get(i))
                    .ok_or("Playlist no longer exists")?;
                let entries: Vec<_> = library::entries(playlist)
                    .iter()
                    .filter_map(|entry| {
                        let media = library::media(entry, &self.sources)?;
                        matches!(media.kind, MediaKind::Audio | MediaKind::Video)
                            .then(|| (media.url, library::label(entry)))
                    })
                    .collect();
                let playback = self.playback.as_mut().ok_or("Playback is unavailable")?;
                if matches!(action, QueuePlaylist) {
                    playback.enqueue_collection(
                        entries.iter().map(|(url, _)| url.clone()).collect(),
                        true,
                        &mut self.ui.windows,
                        &mut self.ui.atlas,
                    )?;
                    playback.remember_titles(entries);
                    self.screens.notice = "Added to the audio queue".into();
                } else {
                    playback.play_playlist(
                        entries,
                        shuffle,
                        self.screens.playlists.looping,
                        &mut self.ui.windows,
                        &mut self.ui.atlas,
                    )?;
                    self.open_screen(ScreenKind::Controls);
                }
            }
            PlaylistLoop => self.screens.playlists.looping = !self.screens.playlists.looping,
            ToggleOpen => self.screens.controls.show_open = !self.screens.controls.show_open,
            ExportPlaylist => {
                let i = self
                    .screens
                    .selected_playlist()
                    .ok_or("Select a playlist")?;
                let list = self
                    .stores
                    .as_ref()
                    .and_then(|s| library::lists(s.library.get()).get(i))
                    .ok_or("Playlist unavailable")?;
                let mut text = String::from("#EXTM3U\n");
                for entry in library::entries(list) {
                    if let Some(url) = library::url(entry) {
                        text.push_str(&format!(
                            "#EXTINF:-1,{}\n{url}\n",
                            library::label(entry).replace(['\r', '\n'], " ")
                        ));
                    }
                }
                arboard::Clipboard::new()
                    .and_then(|mut c| c.set_text(text))
                    .map_err(|e| format!("Clipboard: {e}"))?;
                self.screens.notice = "Playlist copied as M3U".into();
            }
            ImportClipboard => {
                let text = arboard::Clipboard::new()
                    .and_then(|mut c| c.get_text())
                    .map_err(|e| format!("Clipboard: {e}"))?;
                let entries = library::clipboard_entries(&text)?;
                let collections: Vec<_> = entries
                    .iter()
                    .filter_map(library::url)
                    .filter_map(|url| self.sources.identify(url))
                    .filter(|m| m.kind == MediaKind::Collection)
                    .collect();
                if !collections.is_empty() {
                    if entries.len() != 1 {
                        return Err("Import playlist links one at a time; M3U imports should contain individual tracks".into());
                    }
                    if self.import_task.is_some() {
                        return Err("An import is already running".into());
                    }
                    let mut index = 0;
                    self.edit_library(|v| {
                        index = library::create(v, "Imported playlist")?;
                        Ok(())
                    })?;
                    self.screens.select_playlist(index);
                    self.screens.playlists.url = crate::ui::TextField::new(&collections[0].url);
                    return self.start_import();
                }
                let mut index = 0;
                self.edit_library(|v| {
                    index = library::create(v, "Imported playlist")?;
                    library::append(v, index, entries)
                })?;
                self.screens.select_playlist(index);
                self.screens.notice = "Playlist imported from clipboard".into();
            }
            HistoryPlay(queue) => {
                let i = self
                    .screens
                    .selected_history()
                    .ok_or("Select a history entry")?;
                let e = &self
                    .stores
                    .as_ref()
                    .ok_or("Library unavailable")?
                    .library
                    .get()["history"][i];
                let media = library::media(e, &self.sources).ok_or("Unsupported history URL")?;
                self.open_chat_media(media, queue);
                self.screens.notice = "Playback requested".into();
            }
            HistoryRemove => {
                let i = self
                    .screens
                    .selected_history()
                    .ok_or("Select a history entry")?;
                self.edit_library(|v| {
                    let a = v["history"].as_array_mut().ok_or("History unavailable")?;
                    if i < a.len() {
                        a.remove(i);
                    }
                    Ok(())
                })?;
            }
            HistoryClear => {
                self.edit_library(|v| {
                    v["history"] = json!([]);
                    Ok(())
                })?;
                self.screens.notice = "History cleared".into();
            }
            Source(source) => {
                self.screens.history.source = source;
                self.screens.history.rows.reset_scroll();
                self.screens.invalidate();
            }
            SourcePage => {
                self.screens.history.source_page += 1;
                self.screens.invalidate();
            }
            Tab(tab) => {
                self.screens.settings.tab = tab;
                self.screens.open(ScreenKind::Settings);
            }
            Setting(key, value) => self.change_setting(key, value)?,
            ToolStatus => self.start_tools(None)?,
            Redownload(tool) => self.start_tools(Some(tool))?,
            CancelTool => {
                if let Some(task) = &self.tool_task {
                    task.cancel();
                    self.screens.settings.tool_status = "Cancelling tool operation…".into();
                }
            }
            Reveal => self.ui.windows.reveal(),
            WindowReveal(id) => {
                if let Some(w) = self.ui.windows.window_mut(id) {
                    w.hidden = false;
                    w.theatre = false;
                }
            }
            WindowHide(id) => {
                if let Some(w) = self.ui.windows.window_mut(id) {
                    w.hidden = true;
                }
            }
            WindowClose(id) => self.ui.windows.close(id),
            PlayerToggle(id) | PlayerNext(id) => {
                let p = self
                    .playback
                    .as_ref()
                    .and_then(|p| p.get(id))
                    .ok_or("Select an audio or video player")?;
                p.player
                    .control
                    .send(if matches!(action, PlayerToggle(_)) {
                        if p.player.control.snapshot().state
                            == crate::core::player::PlaybackState::Failed
                        {
                            PlayerCommand::Retry
                        } else {
                            PlayerCommand::Toggle
                        }
                    } else {
                        PlayerCommand::Next
                    })?;
            }
            OpenUrl(audio) => {
                let url = self.screens.controls.url.text().trim().to_owned();
                let media = if audio {
                    crate::core::sources::web_url(&url).ok_or("Expected an HTTP(S) URL")?;
                    let mut media = self.sources.identify(&url).unwrap_or(MediaRef {
                        url,
                        kind: MediaKind::Audio,
                        entry_kind: MediaKind::Audio,
                        source_id: "liasmediaplayer:audio".into(),
                        label: "audio".into(),
                    });
                    media.entry_kind = MediaKind::Audio;
                    if media.kind != MediaKind::Collection {
                        media.kind = MediaKind::Audio;
                    }
                    media
                } else {
                    self.sources
                        .identify(&url)
                        .ok_or("Use a supported media URL")?
                };
                if media.kind == MediaKind::Collection {
                    self.screens.notice = self.execute_show(crate::command::ShowCommand {
                        kind: if audio {
                            crate::command::ShowKind::Audio
                        } else {
                            crate::command::ShowKind::Video
                        },
                        url: media.url,
                        new_player: true,
                    });
                    return Ok(());
                }
                self.open_chat_media(media, false);
                self.screens.notice = self
                    .chat
                    .notice
                    .as_ref()
                    .map(|(s, _)| s.clone())
                    .unwrap_or_default();
            }
        }
        Ok(())
    }
    fn start_import(&mut self) -> Result<(), String> {
        if self.import_task.is_some() {
            return Err("An import is already running".into());
        }
        let i = self
            .screens
            .selected_playlist()
            .ok_or("Select a target playlist first")?;
        let media = self
            .sources
            .identify(self.screens.playlists.url.text().trim())
            .filter(|m| m.kind == MediaKind::Collection)
            .ok_or("Enter a supported playlist / album / set URL")?;
        let mut id = String::new();
        self.edit_library(|v| {
            let existing = v["playlists"]
                .get(i)
                .and_then(|p| p["id"].as_str())
                .map(str::to_owned);
            let unique = existing.as_ref().is_some_and(|id| {
                !id.is_empty()
                    && library::lists(v)
                        .iter()
                        .filter(|p| p["id"].as_str() == Some(id))
                        .count()
                        == 1
            });
            let list = v["playlists"]
                .get_mut(i)
                .filter(|p| p.is_object())
                .ok_or("Select a valid playlist")?;
            // Imported Java playlists may have missing/duplicate IDs.
            if !unique {
                list["id"] = json!(format!(
                    "import-target-{}-{i}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                ));
            }
            id = list["id"].as_str().unwrap().into();
            Ok(())
        })?;
        self.import_task = Some((
            id,
            CollectionTask::start(self.chat_tools.clone(), media, false),
        ));
        self.screens.playlists.importing = true;
        self.screens.notice = "Importing collection…".into();
        Ok(())
    }
    fn start_tools(&mut self, download: Option<Tool>) -> Result<(), String> {
        if self.tool_task.is_some() {
            return Err("A tool operation is already running".into());
        }
        self.tool_task = Some(ToolTask::start(self.chat_tools.clone(), download));
        self.screens.settings.tool_busy = true;
        self.screens.settings.tool_status = if download.is_some() {
            "Downloading and validating…"
        } else {
            "Checking binaries…"
        }
        .into();
        Ok(())
    }
    pub fn poll_screen_jobs(&mut self) {
        if let Some(result) = self.tool_task.as_ref().and_then(ToolTask::poll) {
            self.tool_task = None;
            self.screens.settings.tool_busy = false;
            self.screens.settings.tool_status = result;
        }
        if let Some(result) = self.import_task.as_ref().and_then(|(_, task)| task.poll()) {
            let (id, task) = self.import_task.take().unwrap();
            self.screens.playlists.importing = false;
            if task.is_cancelled() {
                return;
            }
            let result = result.and_then(|collection| {
                self.edit_library(|v| {
                    let i = library::lists(v)
                        .iter()
                        .position(|p| p["id"].as_str() == Some(&id))
                        .ok_or("Import target was deleted; nothing added")?;
                    if collection.entries.is_empty() {
                        return Err("Collection contains no tracks".into());
                    }
                    library::append(
                        v,
                        i,
                        collection
                            .entries
                            .into_iter()
                            .map(|m| json!({"url":m.url,"title":m.label,"kind":format!("{:?}",m.kind)}))
                            .collect(),
                    )
                })
            });
            self.screens.notice =
                result.map_or_else(|e| format!("Import: {e}"), |_| "Collection imported".into());
        }
    }
    fn change_setting(&mut self, key: &str, value: Value) -> Result<(), String> {
        let stores = self
            .stores
            .as_mut()
            .ok_or("Settings storage is unavailable")?;
        let mut settings = stores.settings.get().clone();
        settings[key] = value.clone();
        stores
            .settings
            .set(settings.clone())
            .map_err(|e| e.to_string())?;
        match key {
            "theme" => {
                if let Some(theme) = crate::api::themes()
                    .into_iter()
                    .find(|t| Some(t.id.as_str()) == value.as_str())
                {
                    self.ui.windows.theme = theme;
                }
            }
            "hudBadge" => self.set_hud_badge_shown(value.as_bool().unwrap_or(true)),
            "replaceChatLinks" => {
                self.replace_chat_links = value.as_bool();
                self.chat_rewrap_requested = true;
            }
            "chatHoverPreviews" => {
                self.chat_hover_previews = value.as_bool().unwrap_or(true);
                if !self.chat_hover_previews {
                    self.chat
                        .clear_hover(self.previews.as_mut(), &mut self.ui.atlas);
                }
            }
            "preferSystemBinaries" => {
                self.chat_tools.policy.prefer_system = value.as_bool().unwrap_or(true)
            }
            "autoDownloadTools" => {
                self.chat_tools.policy.auto_download = value.as_bool().unwrap_or(true)
            }
            _ => {}
        }
        if let Some(p) = &mut self.playback {
            p.configure(&settings);
            p.set_tools(self.chat_tools.clone());
            if key == "masterVolume" {
                p.set_volume(value.as_f64().unwrap_or(1.) as f32)?;
            }
        }
        self.screens.invalidate();
        self.screens.notice = if matches!(key, "preferSystemBinaries" | "autoDownloadTools") {
            "Saved. Tool policy applies to new players and new background jobs."
        } else {
            "Settings saved"
        }
        .into();
        Ok(())
    }
}
