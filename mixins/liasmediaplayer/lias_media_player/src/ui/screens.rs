//! Full-screen views; widgets emit intents and Runtime owns persistence and workers.
use super::{
    WindowManager,
    theme::{ThemeRole as R, ThemeSpec},
    widgets::*,
};
use crate::{
    core::{library, playback::PlaybackRegistry, sources::SourceRegistry, tools::Tool},
    platform::{Canvas, InputEvent, Key, Rect},
};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenKind {
    Controls,
    Playlists,
    History,
    Settings,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    Appearance,
    Chat,
    Playback,
    Tools,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ScreenAction {
    Navigate(ScreenKind),
    Close,
    Create,
    Rename,
    DeletePlaylist,
    AddUrl,
    Import,
    CancelImport,
    RemoveEntry,
    Reorder(usize, usize),
    PlayEntry(bool),
    QueuePlaylist,
    PlayPlaylist(bool),
    PlaylistLoop,
    ImportClipboard,
    ExportPlaylist,
    ToggleOpen,
    HistoryPlay(bool),
    HistoryRemove,
    HistoryClear,
    Setting(&'static str, Value),
    ToolStatus,
    Redownload(Tool),
    CancelTool,
    Reveal,
    WindowReveal(u64),
    WindowHide(u64),
    WindowClose(u64),
    PlayerToggle(u64),
    PlayerNext(u64),
    OpenUrl(bool),
    Tab(SettingsTab),
    Source(Option<String>),
    SourcePage,
}
pub struct PlaylistScreen {
    pub names: ListView,
    pub tracks: ListView,
    pub name: TextField,
    pub new_name: TextField,
    pub search: TextField,
    pub entry_indices: Vec<usize>,
    pub looping: bool,
    delete_armed: Option<std::time::Instant>,
    pub url: TextField,
    pub importing: bool,
    last_selected: Option<usize>,
}
pub struct HistoryScreen {
    pub search: TextField,
    pub rows: ListView,
    pub indices: Vec<usize>,
    pub source: Option<String>,
    sources: Vec<(String, String)>,
    pub source_page: usize,
    keys: Vec<String>,
}
pub struct SettingsScreen {
    pub tab: SettingsTab,
    pub tool_status: String,
    pub tool_busy: bool,
}
pub struct MediaControlsScreen {
    pub url: TextField,
    pub rows: ListView,
    pub ids: Vec<u64>,
    pub show_open: bool,
    pub hidden_count: usize,
}
struct ActionButton {
    button: Button,
    action: ScreenAction,
    secondary: bool,
}
pub struct Screens {
    pub active: Option<ScreenKind>,
    pub playlists: PlaylistScreen,
    pub history: HistoryScreen,
    pub settings: SettingsScreen,
    pub controls: MediaControlsScreen,
    pub notice: String,
    buttons: Vec<ActionButton>,
    input: InputState,
    dirty: bool,
    viewport: [f32; 2],
    scroll: f32,
    max_scroll: f32,
    content: Rect,
}
impl Default for Screens {
    fn default() -> Self {
        let mut tracks = ListView::new(vec![]);
        tracks.reorderable = true;
        tracks.removable = true;
        tracks.row_height = 18.;
        let field = |hint: &str| {
            let mut f = TextField::new("");
            f.placeholder = hint.into();
            f
        };
        Self {
            active: None,
            playlists: PlaylistScreen {
                names: ListView::new(vec![]),
                tracks,
                name: field("Playlist name"),
                new_name: field("New playlist"),
                search: field("Search tracks"),
                entry_indices: Vec::new(),
                looping: false,
                delete_armed: None,
                url: field("Media or collection URL (Ctrl+V to paste)"),
                importing: false,
                last_selected: None,
            },
            history: HistoryScreen {
                search: field("Search title or URL"),
                rows: ListView::new(vec![]),
                indices: vec![],
                source: None,
                sources: vec![],
                source_page: 0,
                keys: vec![],
            },
            settings: SettingsScreen {
                tab: SettingsTab::Appearance,
                tool_status: "Check status to locate installed tools".into(),
                tool_busy: false,
            },
            controls: MediaControlsScreen {
                url: field("Media URL (Ctrl+V to paste)"),
                rows: ListView::new(vec![]),
                ids: vec![],
                show_open: false,
                hidden_count: 0,
            },
            notice: String::new(),
            buttons: vec![],
            input: InputState::default(),
            dirty: true,
            viewport: [0.; 2],
            scroll: 0.,
            max_scroll: 0.,
            content: Rect::default(),
        }
    }
}
impl Screens {
    pub fn open(&mut self, kind: ScreenKind) {
        self.active = Some(kind);
        self.scroll = 0.;
        self.cancel_input();
        self.dirty = true;
    }
    pub fn close(&mut self) {
        self.active = None;
        self.cancel_input();
    }
    pub fn cancel_input(&mut self) {
        self.input.clear();
        self.playlists.tracks.cancel_drag();
    }
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
    pub fn selected_playlist(&self) -> Option<usize> {
        self.playlists.names.selected
    }
    pub fn selected_entry(&self) -> Option<usize> {
        self.playlists
            .tracks
            .selected
            .and_then(|i| self.playlists.entry_indices.get(i).copied())
    }
    pub fn selected_history(&self) -> Option<usize> {
        self.history
            .rows
            .selected
            .and_then(|i| self.history.indices.get(i).copied())
    }
    pub fn select_playlist(&mut self, i: usize) {
        self.playlists.names.selected = Some(i);
        self.playlists.search = TextField::new("");
        self.playlists.search.placeholder = "Search tracks".into();
        self.playlists.delete_armed = None;
        self.playlists.tracks.reset_scroll();
        self.dirty = true;
    }
    pub fn sync(
        &mut self,
        lib: &Value,
        sources: &SourceRegistry,
        windows: &WindowManager,
        playback: Option<&PlaybackRegistry>,
    ) {
        self.playlists.names.items = library::lists(lib)
            .iter()
            .map(|p| format!("{} ({})", library::name(p), library::entries(p).len()))
            .collect();
        if self
            .playlists
            .names
            .selected
            .is_some_and(|i| i >= self.playlists.names.items.len())
        {
            self.playlists.names.selected = None;
            self.playlists.tracks.reset_scroll();
        }
        if self.playlists.last_selected != self.selected_playlist() {
            self.playlists.last_selected = self.selected_playlist();
            if let Some(list) = self
                .selected_playlist()
                .and_then(|i| library::lists(lib).get(i))
            {
                self.playlists.name = TextField::new(library::name(list));
                self.playlists.name.max_chars = 128;
            }
        }
        let query = self.playlists.search.text().trim().to_lowercase();
        let entries = self
            .selected_playlist()
            .and_then(|i| library::lists(lib).get(i))
            .map(library::entries)
            .unwrap_or(&[]);
        let indices: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                query.is_empty()
                    || library::label(e).to_lowercase().contains(&query)
                    || library::url(e).is_some_and(|s| s.to_lowercase().contains(&query))
            })
            .map(|(i, _)| i)
            .collect();
        if indices != self.playlists.entry_indices {
            self.playlists.tracks.reset_scroll();
        }
        self.playlists.tracks.items = indices
            .iter()
            .map(|&i| format!("{}. {}", i + 1, library::label(&entries[i])))
            .collect();
        self.playlists.entry_indices = indices;
        self.playlists.tracks.reorderable = query.is_empty();
        let indices = library::history_indices(
            lib,
            self.history.search.text(),
            self.history.source.as_deref(),
            sources,
        );
        let keys: Vec<_> = indices
            .iter()
            .map(|&i| library::url(&lib["history"][i]).unwrap_or("").to_owned())
            .collect();
        if indices != self.history.indices || keys != self.history.keys {
            self.history.rows.reset_scroll();
        }
        self.history.keys = keys;
        self.history.rows.items = indices
            .iter()
            .map(|&i| library::label(&lib["history"][i]))
            .collect();
        self.history.indices = indices;
        let mut tabs: Vec<_> = lib["history"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| library::media(e, sources))
            .map(|m| (m.source_id, m.label))
            .collect();
        tabs.sort();
        tabs.dedup_by(|a, b| a.0 == b.0);
        if tabs != self.history.sources {
            self.history.sources = tabs;
            self.dirty = true;
        }
        let ids: Vec<_> = windows.windows().iter().map(|w| w.id).collect();
        let selected = self
            .controls
            .rows
            .selected
            .and_then(|i| self.controls.ids.get(i))
            .copied();
        self.controls.rows.selected = selected.and_then(|id| ids.iter().position(|i| *i == id));
        self.controls.ids = ids;
        self.controls.hidden_count = windows.windows().iter().filter(|w| w.hidden).count();
        self.controls.rows.items = windows
            .windows()
            .iter()
            .map(|w| {
                let state = playback
                    .and_then(|p| p.get(w.id))
                    .map(|p| {
                        let s = p.player.control.snapshot();
                        format!(
                            " · {:?} · {:.0}s · {} queued",
                            s.state,
                            s.position_secs,
                            s.queue.len()
                        )
                    })
                    .unwrap_or_default();
                format!(
                    "{}{}{}",
                    if w.hidden { "[Hidden] " } else { "" },
                    w.title,
                    state
                )
            })
            .collect();
    }
    fn add(&mut self, label: impl Into<String>, action: ScreenAction, secondary: bool) {
        self.buttons.push(ActionButton {
            button: Button::new(label),
            action,
            secondary,
        });
    }
    fn build(&mut self, settings: &Value) {
        if self
            .buttons
            .iter()
            .any(|b| Some(b.button.id()) == self.input.focus)
        {
            self.input.clear();
        }
        self.buttons.clear();
        for (label, kind) in [
            ("Controls", ScreenKind::Controls),
            ("Playlists", ScreenKind::Playlists),
            ("History", ScreenKind::History),
            ("Settings", ScreenKind::Settings),
        ] {
            self.add(label.to_owned(), ScreenAction::Navigate(kind), false);
        }
        self.add("Close", ScreenAction::Close, false);
        match self.active {
            Some(ScreenKind::Playlists) => {
                for (label, a) in [
                    ("+", ScreenAction::Create),
                    ("Rename", ScreenAction::Rename),
                    ("Delete", ScreenAction::DeletePlaylist),
                    ("Add", ScreenAction::AddUrl),
                    ("Cancel import", ScreenAction::CancelImport),
                    ("Play", ScreenAction::PlayPlaylist(false)),
                    ("Shuffle", ScreenAction::PlayPlaylist(true)),
                    ("Loop", ScreenAction::PlaylistLoop),
                    ("Queue", ScreenAction::QueuePlaylist),
                    ("Export", ScreenAction::ExportPlaylist),
                    ("Import", ScreenAction::ImportClipboard),
                ] {
                    self.add(label, a, false);
                }
            }
            Some(ScreenKind::History) => {
                self.add("All sources", ScreenAction::Source(None), true);
                let pages = self.history.sources.len().div_ceil(3).max(1);
                self.history.source_page %= pages;
                for (id, label) in self
                    .history
                    .sources
                    .clone()
                    .into_iter()
                    .skip(self.history.source_page * 3)
                    .take(3)
                {
                    let label = if self.history.source.as_ref() == Some(&id) {
                        format!("[{label}]")
                    } else {
                        label
                    };
                    self.add(label, ScreenAction::Source(Some(id)), true);
                }
                if pages > 1 {
                    self.add(
                        format!("Sources {}/{} >", self.history.source_page + 1, pages),
                        ScreenAction::SourcePage,
                        true,
                    );
                }
                for (label, a) in [
                    ("Play", ScreenAction::HistoryPlay(false)),
                    ("Queue", ScreenAction::HistoryPlay(true)),
                    ("Remove", ScreenAction::HistoryRemove),
                    ("Clear history", ScreenAction::HistoryClear),
                ] {
                    self.add(label, a, false);
                }
            }
            Some(ScreenKind::Settings) => {
                for (label, tab) in [
                    ("Appearance", SettingsTab::Appearance),
                    ("Chat", SettingsTab::Chat),
                    ("Playback", SettingsTab::Playback),
                    ("Tools", SettingsTab::Tools),
                ] {
                    self.add(
                        if self.settings.tab == tab {
                            format!("[{label}]")
                        } else {
                            label.into()
                        },
                        ScreenAction::Tab(tab),
                        true,
                    );
                }
                match self.settings.tab {
                    SettingsTab::Appearance => {
                        for theme in crate::api::themes() {
                            self.add(
                                format!("Theme: {}", theme.name),
                                ScreenAction::Setting("theme", json!(theme.id)),
                                false,
                            );
                        }
                        self.toggle(settings, "hudBadge", "HUD status badge", true);
                    }
                    SettingsTab::Chat => {
                        self.toggle(settings, "replaceChatLinks", "Replace chat links", true);
                        self.toggle(settings, "chatHoverPreviews", "Hover previews", true);
                    }
                    SettingsTab::Playback => {
                        self.cycle(
                            settings,
                            "masterVolume",
                            "Volume",
                            &[json!(0.0), json!(0.25), json!(0.5), json!(0.75), json!(1.0)],
                            json!(1.0),
                        );
                        self.cycle(
                            settings,
                            "defaultQuality",
                            "New player quality",
                            &[json!(360), json!(480), json!(720), json!(1080)],
                            json!(720),
                        );
                        self.cycle(
                            settings,
                            "videoFps",
                            "New player FPS",
                            &[json!(30), json!(60)],
                            json!(30),
                        );
                        self.cycle(
                            settings,
                            "maxConcurrentVideos",
                            "Video limit",
                            &[json!(1), json!(2), json!(3), json!(4)],
                            json!(2),
                        );
                        self.cycle(
                            settings,
                            "backgroundMode",
                            "Background",
                            &[json!("sound_only_after_5s"), json!("keep_video")],
                            json!("sound_only_after_5s"),
                        );
                    }
                    SettingsTab::Tools => {
                        self.toggle(settings, "preferSystemBinaries", "Search PATH", true);
                        self.toggle(
                            settings,
                            "autoDownloadTools",
                            "Download missing tools",
                            true,
                        );
                        self.add("Check binary status", ScreenAction::ToolStatus, false);
                        self.add(
                            "Redownload yt-dlp",
                            ScreenAction::Redownload(Tool::YtDlp),
                            false,
                        );
                        self.add(
                            "Redownload FFmpeg",
                            ScreenAction::Redownload(Tool::Ffmpeg),
                            false,
                        );
                        self.add("Cancel tool operation", ScreenAction::CancelTool, false);
                    }
                }
            }
            Some(ScreenKind::Controls) => {
                self.add("Audio", ScreenAction::OpenUrl(true), false);
                self.add("Video", ScreenAction::OpenUrl(false), false);
                self.add("Show hidden", ScreenAction::Reveal, false);
                self.add("Open link", ScreenAction::ToggleOpen, false);
            }
            None => {}
        }
        self.dirty = false;
    }
    fn toggle(&mut self, s: &Value, key: &'static str, label: &str, default: bool) {
        let v = setting(s, key).as_bool().unwrap_or(default);
        self.add(
            format!("{label}: {}", if v { "ON" } else { "OFF" }),
            ScreenAction::Setting(key, json!(!v)),
            false,
        );
    }
    fn cycle(
        &mut self,
        s: &Value,
        key: &'static str,
        label: &str,
        values: &[Value],
        default: Value,
    ) {
        let value = setting(s, key);
        let v = if value.is_null() { &default } else { value };
        let next = values
            .iter()
            .position(|x| x == v)
            .map_or(0, |i| (i + 1) % values.len());
        self.add(
            format!(
                "{label}: {}",
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            ),
            ScreenAction::Setting(key, values[next].clone()),
            false,
        );
    }
    pub fn draw(
        &mut self,
        c: &mut dyn Canvas,
        theme: &ThemeSpec,
        mouse: Option<[f32; 2]>,
        settings: &Value,
    ) {
        if self.active.is_none() {
            return;
        }
        if self.dirty {
            self.build(settings);
        }
        if self.viewport != c.size() {
            self.cancel_input();
            self.viewport = c.size();
        }
        let list = self.selected_playlist().is_some();
        let track = self.selected_entry().is_some();
        let history = self.selected_history().is_some();
        let player = self.controls.rows.selected.is_some();
        for b in &mut self.buttons {
            b.button.enabled = match b.action {
                ScreenAction::Rename
                | ScreenAction::DeletePlaylist
                | ScreenAction::AddUrl
                | ScreenAction::QueuePlaylist
                | ScreenAction::PlayPlaylist(_)
                | ScreenAction::ExportPlaylist
                | ScreenAction::PlaylistLoop => list,
                ScreenAction::PlayEntry(_) | ScreenAction::RemoveEntry => list && track,
                ScreenAction::Import => list && !self.playlists.importing,
                ScreenAction::CancelImport => self.playlists.importing,
                ScreenAction::HistoryPlay(_) | ScreenAction::HistoryRemove => history,
                ScreenAction::HistoryClear => !self.history.rows.items.is_empty(),
                ScreenAction::ToolStatus | ScreenAction::Redownload(_) => !self.settings.tool_busy,
                ScreenAction::CancelTool => self.settings.tool_busy,
                ScreenAction::WindowReveal(_)
                | ScreenAction::WindowHide(_)
                | ScreenAction::WindowClose(_)
                | ScreenAction::PlayerToggle(_)
                | ScreenAction::PlayerNext(_) => player,
                _ => true,
            };
        }
        if self.active == Some(ScreenKind::Controls) {
            self.draw_controls(c, theme, mouse);
            return;
        }
        if self.active == Some(ScreenKind::Playlists) {
            self.draw_playlists(c, theme, mouse);
            return;
        }
        let [w, h] = self.viewport;
        let r = Rect::new(0., 0., w, h);
        c.fill(r, theme.color(R::WindowBg) | 0xff000000);
        c.push_clip(r);
        c.text("Lia's Media Player", [8., 7.], theme.color(R::Text), true);
        let mut x = 8.;
        let mut y = 23.;
        for b in self.buttons.iter_mut().take(4) {
            let width = (c.text_width(&b.button.label) + 16.).min((w - 16.).max(0.));
            if x + width > w - 8. && x > 8. {
                x = 8.;
                y += 24.;
            }
            b.button.layout(Rect::new(x, y, width, 20.));
            x += width + 4.;
        }
        y += 26.;
        if let Some(close) = self.buttons.get_mut(4) {
            close
                .button
                .layout(Rect::new((w - 50.).max(0.), 3., 42., 17.));
        }
        let ctx = DrawContext {
            theme,
            mouse,
            focus: self.input.focus,
        };
        for b in self.buttons.iter().take(5) {
            b.button.draw(c, &ctx);
            if matches!(b.action, ScreenAction::Navigate(kind) if Some(kind) == self.active) {
                let r = b.button.rect();
                c.fill(
                    Rect::new(r.x, r.bottom() - 2., r.w, 2.),
                    theme.color(R::Fill),
                );
            }
        }
        self.content = Rect::new(8., y, (w - 16.).max(0.), (h - y - 36.).max(0.));
        c.push_clip(self.content);
        let base = y;
        if self.active == Some(ScreenKind::Settings) {
            y -= self.scroll;
        }
        // Secondary tabs wrap without losing access on a small GUI viewport.
        x = 8.;
        let mut secondary = false;
        for b in self.buttons.iter_mut().skip(5).filter(|b| b.secondary) {
            let width = (c.text_width(&b.button.label) + 16.).min((w - 16.).max(0.));
            if x + width > w - 8. && x > 8. {
                x = 8.;
                y += 24.;
            }
            b.button.layout(Rect::new(x, y, width, 20.));
            x += width + 4.;
            secondary = true;
        }
        if secondary {
            y += 26.;
        }
        match self.active {
            Some(ScreenKind::Playlists) => {
                // Pair each field with its actions instead of one undifferentiated toolbar.
                let mut right = w - 8.;
                for action in [
                    ScreenAction::DeletePlaylist,
                    ScreenAction::Rename,
                    ScreenAction::Create,
                ] {
                    if let Some(b) = self.buttons.iter_mut().find(|b| b.action == action) {
                        let width = c.text_width(&b.button.label) + 12.;
                        right -= width;
                        b.button.layout(Rect::new(right, y, width, 20.));
                        right -= 4.;
                    }
                }
                self.playlists
                    .name
                    .arrange(c, Rect::new(8., y, (right - 8.).max(0.), 20.));
                y += 26.;
                let action = if self.playlists.importing {
                    ScreenAction::CancelImport
                } else {
                    ScreenAction::AddUrl
                };
                let mut right = w - 8.;
                for b in self.buttons.iter_mut().filter(|b| {
                    matches!(b.action, ScreenAction::AddUrl | ScreenAction::CancelImport)
                }) {
                    if b.action == action {
                        let width = c.text_width(&b.button.label) + 16.;
                        right -= width;
                        b.button.layout(Rect::new(right, y, width, 20.));
                        right -= 4.;
                    } else {
                        b.button.layout(Rect::default());
                    }
                }
                self.playlists
                    .url
                    .arrange(c, Rect::new(8., y, (right - 8.).max(0.), 20.));
                y += 26.;
            }
            Some(ScreenKind::History) => {
                self.history
                    .search
                    .arrange(c, Rect::new(8., y, (w - 16.).max(0.), 20.));
                y += 26.;
            }
            Some(ScreenKind::Controls) => {
                self.controls
                    .url
                    .arrange(c, Rect::new(8., y, (w - 16.).max(0.), 20.));
                y += 26.;
            }
            _ => {}
        }
        x = 8.;
        let playlists = self.active == Some(ScreenKind::Playlists);
        for b in self.buttons.iter_mut().skip(5).filter(|b| !b.secondary) {
            if playlists
                && matches!(
                    b.action,
                    ScreenAction::Create
                        | ScreenAction::Rename
                        | ScreenAction::DeletePlaylist
                        | ScreenAction::AddUrl
                        | ScreenAction::CancelImport
                )
            {
                continue;
            }
            let width = (c.text_width(&b.button.label) + 16.).min((w - 16.).max(0.));
            if self.active == Some(ScreenKind::Settings) {
                b.button.layout(Rect::new(8., y, width, 20.));
                y += 24.;
            } else {
                if x + width > w - 8. && x > 8. {
                    x = 8.;
                    y += 24.;
                }
                b.button.layout(Rect::new(x, y, width, 20.));
                x += width + 4.;
            }
        }
        if self.active != Some(ScreenKind::Settings) {
            y += 26.;
        }
        let ctx = DrawContext {
            theme,
            mouse,
            focus: self.input.focus,
        };
        for b in self.buttons.iter().skip(5) {
            if b.button.rect().w <= 0. {
                continue;
            }
            b.button.draw(c, &ctx);
            if matches!(
                b.action,
                ScreenAction::OpenUrl(_)
                    | ScreenAction::AddUrl
                    | ScreenAction::PlayEntry(false)
                    | ScreenAction::QueuePlaylist
            ) && b.button.enabled
            {
                let r = b.button.rect();
                c.fill(
                    Rect::new(r.x, r.bottom() - 2., r.w, 2.),
                    theme.color(R::Fill),
                );
            }
        }
        let rows = Rect::new(
            8.,
            y,
            (w - 16.).max(0.),
            (self.content.bottom() - y).max(0.),
        );
        match self.active {
            Some(ScreenKind::Playlists) => {
                self.playlists.name.draw(c, &ctx);
                self.playlists.url.draw(c, &ctx);
                let left = (rows.w * 0.32).max(0.);
                self.playlists
                    .names
                    .layout(Rect::new(rows.x, rows.y, left, rows.h));
                self.playlists.tracks.layout(Rect::new(
                    rows.x + left + 4.,
                    rows.y,
                    (rows.w - left - 4.).max(0.),
                    rows.h,
                ));
                self.playlists.names.draw(c, &ctx);
                self.playlists.tracks.draw(c, &ctx);
                if self.playlists.names.items.is_empty() {
                    super::text::paragraph(
                        c,
                        rows.inset(8.),
                        "Your library is empty. Give your playlist a name and choose Create.",
                        theme.color(R::TextSubtle),
                    );
                } else if self.playlists.tracks.items.is_empty() {
                    let r = self.playlists.tracks.rect().inset(8.);
                    super::text::paragraph(
                        c,
                        r,
                        "Paste a video or playlist URL above, then choose Add URL. YouTube playlists are imported automatically.",
                        theme.color(R::TextSubtle),
                    );
                }
            }
            Some(ScreenKind::History) => {
                self.history.search.draw(c, &ctx);
                self.history.rows.layout(rows);
                self.history.rows.draw(c, &ctx);
            }
            Some(ScreenKind::Controls) => {
                self.controls.url.draw(c, &ctx);
                self.controls.rows.layout(rows);
                self.controls.rows.draw(c, &ctx);
                if self.controls.ids.is_empty() {
                    super::text::paragraph(
                        c,
                        rows.inset(12.),
                        "Nothing playing yet. Paste a media or YouTube playlist link above to get started.",
                        theme.color(R::TextSubtle),
                    );
                }
            }
            Some(ScreenKind::Settings) => {
                if self.settings.tab == SettingsTab::Tools {
                    for line in self.settings.tool_status.lines() {
                        for line in wrap(c, line, (w - 20.).max(1.)) {
                            c.text(&line, [8., y + 4.], theme.color(R::TextSubtle), false);
                            y += 14.;
                        }
                    }
                }
                self.max_scroll = (y + self.scroll - base - self.content.h + 16.).max(0.);
                self.scroll = self.scroll.min(self.max_scroll);
            }
            None => {}
        }
        c.pop_clip();
        if self.active == Some(ScreenKind::Settings) && self.max_scroll > 0. && self.content.h > 0.
        {
            let r = self.content;
            let thumb = (r.h * r.h / (r.h + self.max_scroll)).max(6.).min(r.h);
            c.fill(
                Rect::new(r.right() - 3., r.y, 3., r.h),
                theme.color(R::ScrollTrack),
            );
            c.fill(
                Rect::new(
                    r.right() - 3.,
                    r.y + self.scroll / self.max_scroll * (r.h - thumb),
                    3.,
                    thumb,
                ),
                theme.color(R::ScrollThumb),
            );
        }
        let hint = if self.notice.is_empty() {
            match self.active {
                Some(ScreenKind::Playlists) => {
                    "Add URL accepts videos and playlists. Drag tracks to reorder. Ctrl+V: paste."
                }
                Some(ScreenKind::Controls) => {
                    "Select a player for controls. M / K / N / P / H are rebindable in Options > Mods."
                }
                Some(ScreenKind::History) => {
                    "Search titles / URLs and choose a source. Select a result, then Play or Queue."
                }
                Some(ScreenKind::Settings) if self.max_scroll > 0. => {
                    "Scroll for more settings. Changes save automatically."
                }
                _ => "Changes save automatically. Escape returns to the game.",
            }
        } else {
            &self.notice
        };
        let footer = Rect::new(8., (h - 32.).max(0.), (w - 16.).max(0.), 30.);
        c.fill(footer, theme.color(R::BannerBg));
        super::text::paragraph(c, footer.inset(3.), hint, theme.color(R::TextSubtle));
        c.pop_clip();
    }
    fn place_action(&mut self, action: ScreenAction, rect: Rect) {
        if let Some(b) = self.buttons.iter_mut().find(|b| b.action == action) {
            b.button.layout(rect);
        }
    }
    fn draw_controls(&mut self, c: &mut dyn Canvas, theme: &ThemeSpec, mouse: Option<[f32; 2]>) {
        let [w, h] = self.viewport;
        self.content = Rect::new(0., 0., w, h);
        for b in &mut self.buttons {
            b.button.layout(Rect::default());
        }
        self.controls.rows.layout(Rect::default());
        let width = (w - 12.).clamp(0., 530.);
        let x = (w - width) / 2.;
        let y = (h - 25.).max(0.);
        let slot = (width - 16.).max(0.) / 5.;
        for (i, action) in [
            ScreenAction::ToggleOpen,
            ScreenAction::Navigate(ScreenKind::Playlists),
            ScreenAction::Navigate(ScreenKind::History),
            ScreenAction::Navigate(ScreenKind::Settings),
            ScreenAction::Close,
        ]
        .into_iter()
        .enumerate()
        {
            self.place_action(action, Rect::new(x + i as f32 * (slot + 4.), y, slot, 18.));
        }
        let opening = self.controls.show_open || self.controls.ids.is_empty();
        if opening {
            let row = Rect::new(x, (y - 24.).max(0.), width, 20.);
            c.fill(row, theme.color(R::PanelBg));
            self.controls
                .url
                .arrange(c, Rect::new(row.x, row.y, (width - 98.).max(0.), 20.));
            self.place_action(
                ScreenAction::OpenUrl(true),
                Rect::new(row.right() - 94., row.y, 45., 20.),
            );
            self.place_action(
                ScreenAction::OpenUrl(false),
                Rect::new(row.right() - 45., row.y, 45., 20.),
            );
        } else {
            self.controls.url.layout(Rect::default());
        }
        if self.controls.hidden_count > 0 {
            let row = Rect::new(x, (y - if opening { 48. } else { 24. }).max(0.), 100., 18.);
            self.place_action(ScreenAction::Reveal, row);
            if let Some(b) = self
                .buttons
                .iter_mut()
                .find(|b| b.action == ScreenAction::Reveal)
            {
                b.button.label = format!("Show hidden ({})", self.controls.hidden_count);
            }
        }
        let ctx = DrawContext {
            theme,
            mouse,
            focus: self.input.focus,
        };
        if self.controls.ids.is_empty() && self.notice.is_empty() {
            let line = "Paste a link below. Audio opens a compact music bar.";
            let label = super::text::ellipsize(c, line, width);
            c.text(
                &label,
                [x, (y - 40.).max(0.)],
                theme.color(R::TextSubtle),
                true,
            );
        }
        for b in &self.buttons {
            if b.button.rect().w > 0. {
                b.button.draw(c, &ctx);
            }
        }
        if opening {
            self.controls.url.draw(c, &ctx);
        }
        if !self.notice.is_empty() {
            let text = super::text::ellipsize(c, &self.notice, width);
            c.text(
                &text,
                [x, (y - if opening { 40. } else { 16. }).max(0.)],
                theme.color(R::TextSubtle),
                true,
            );
        }
    }
    fn draw_playlists(&mut self, c: &mut dyn Canvas, theme: &ThemeSpec, mouse: Option<[f32; 2]>) {
        let [w, h] = self.viewport;
        let width = (w - 12.).clamp(0., 560.);
        let height = (h - 12.).clamp(0., 340.);
        let panel = Rect::new((w - width) / 2., (h - height) / 2., width, height);
        self.content = panel;
        c.fill(Rect::new(0., 0., w, h), 0x66000000);
        c.fill(panel, theme.color(R::WindowBg) | 0xff000000);
        c.border(panel, theme.color(R::Border));
        c.push_clip(panel);
        for b in &mut self.buttons {
            b.button.layout(Rect::default());
        }
        c.text(
            "Playlists",
            [panel.x + 6., panel.y + 7.],
            theme.color(R::Text),
            true,
        );
        let mut right = panel.right() - 5.;
        for (action, label) in [
            (ScreenAction::Close, "Done"),
            (ScreenAction::Navigate(ScreenKind::Settings), "Settings"),
            (ScreenAction::Navigate(ScreenKind::History), "History"),
            (ScreenAction::Navigate(ScreenKind::Controls), "Player"),
        ] {
            let bw = c.text_width(label) + 10.;
            right -= bw;
            if let Some(b) = self.buttons.iter_mut().find(|b| b.action == action) {
                b.button.label = label.into();
                b.button.layout(Rect::new(right, panel.y + 3., bw, 18.));
            }
            right -= 3.;
        }
        let y = panel.y + 28.;
        let bottom = panel.bottom() - 21.;
        let left = Rect::new(
            panel.x + 6.,
            y,
            (width * 0.29).max(0.),
            (bottom - y).max(0.),
        );
        let rx = left.right() + 9.;
        let rw = (panel.right() - 6. - rx).max(0.);
        let count = super::text::ellipsize(
            c,
            &format!("Your playlists · {}", self.playlists.names.items.len()),
            left.w,
        );
        c.text(&count, [left.x, left.y], theme.color(R::TextSubtle), false);
        self.playlists.names.row_height = 18.;
        self.playlists.names.layout(Rect::new(
            left.x,
            left.y + 14.,
            left.w,
            (left.h - 58.).max(0.),
        ));
        self.playlists.new_name.arrange(
            c,
            Rect::new(left.x, bottom - 40., (left.w - 22.).max(0.), 18.),
        );
        self.place_action(
            ScreenAction::Create,
            Rect::new(left.right() - 18., bottom - 40., 18., 18.),
        );
        self.place_action(
            ScreenAction::ImportClipboard,
            Rect::new(left.x, bottom - 18., left.w, 18.),
        );
        let chosen = self.selected_playlist().is_some();
        if chosen {
            self.playlists
                .name
                .arrange(c, Rect::new(rx, y, (rw - 52.).max(0.), 18.));
            self.place_action(ScreenAction::Rename, Rect::new(rx + rw - 48., y, 48., 18.));
            self.playlists
                .search
                .arrange(c, Rect::new(rx, y + 22., rw, 18.));
            let count = format!(
                "{} tracks · Audio playback",
                self.playlists.tracks.items.len()
            );
            let label = super::text::ellipsize(c, &count, rw);
            c.text(&label, [rx, y + 44.], theme.color(R::TextSubtle), false);
            let rows = Rect::new(rx, y + 58., rw, (bottom - 68. - y - 58.).max(0.));
            self.playlists.tracks.layout(rows);
            self.playlists
                .url
                .arrange(c, Rect::new(rx, bottom - 64., (rw - 38.).max(0.), 18.));
            self.place_action(
                if self.playlists.importing {
                    ScreenAction::CancelImport
                } else {
                    ScreenAction::AddUrl
                },
                Rect::new(rx + rw - 34., bottom - 64., 34., 18.),
            );
            for (row, actions) in [
                [
                    ScreenAction::PlayPlaylist(false),
                    ScreenAction::PlayPlaylist(true),
                    ScreenAction::PlaylistLoop,
                ],
                [
                    ScreenAction::QueuePlaylist,
                    ScreenAction::ExportPlaylist,
                    ScreenAction::DeletePlaylist,
                ],
            ]
            .into_iter()
            .enumerate()
            {
                let slot = (rw - 6.).max(0.) / 3.;
                for (i, action) in actions.into_iter().enumerate() {
                    self.place_action(
                        action,
                        Rect::new(
                            rx + i as f32 * (slot + 3.),
                            bottom - 42. + row as f32 * 22.,
                            slot,
                            18.,
                        ),
                    );
                }
            }
            if let Some(b) = self
                .buttons
                .iter_mut()
                .find(|b| b.action == ScreenAction::PlaylistLoop)
            {
                b.button.label = if self.playlists.looping {
                    "Loop: on"
                } else {
                    "Loop: off"
                }
                .into();
            }
            if let Some(b) = self
                .buttons
                .iter_mut()
                .find(|b| b.action == ScreenAction::DeletePlaylist)
            {
                b.button.label = if self
                    .playlists
                    .delete_armed
                    .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3))
                {
                    "Confirm?"
                } else {
                    "Delete"
                }
                .into();
            }
        } else {
            for field in [
                &mut self.playlists.name,
                &mut self.playlists.search,
                &mut self.playlists.url,
            ] {
                field.layout(Rect::default());
            }
            self.playlists.tracks.layout(Rect::default());
            super::text::paragraph(
                c,
                Rect::new(rx, y + 20., rw, (bottom - y - 20.).max(0.)),
                if self.playlists.names.items.is_empty() {
                    "Create a playlist on the left, then add your music links here. You can also import an M3U playlist from the clipboard."
                } else {
                    "Select a playlist on the left to view and play its tracks."
                },
                theme.color(R::TextSubtle),
            );
        }
        let ctx = DrawContext {
            theme,
            mouse,
            focus: self.input.focus,
        };
        self.playlists.names.draw(c, &ctx);
        self.playlists.new_name.draw(c, &ctx);
        if chosen {
            self.playlists.name.draw(c, &ctx);
            self.playlists.search.draw(c, &ctx);
            self.playlists.url.draw(c, &ctx);
            self.playlists.tracks.draw(c, &ctx);
            if self.playlists.tracks.items.is_empty() {
                super::text::paragraph(
                    c,
                    self.playlists.tracks.rect().inset(4.),
                    if self.playlists.search.text().is_empty() {
                        "Paste a link below to add tracks."
                    } else {
                        "No matching tracks."
                    },
                    theme.color(R::TextDim),
                );
            }
        }
        for b in &self.buttons {
            if b.button.rect().w > 0. {
                b.button.draw(c, &ctx);
                if matches!(b.action, ScreenAction::PlayPlaylist(_)) {
                    let r = b.button.rect();
                    c.fill(
                        Rect::new(r.x, r.bottom() - 1., r.w, 1.),
                        theme.color(R::Fill),
                    );
                }
            }
        }
        let hint = if self.notice.is_empty() {
            "Play opens audio · Double-click a track to play · Drag to reorder"
        } else {
            &self.notice
        };
        let label = super::text::ellipsize(c, hint, panel.w - 12.);
        c.text(
            &label,
            [panel.x + 6., panel.bottom() - 13.],
            theme.color(R::TextSubtle),
            false,
        );
        c.pop_clip();
    }
    fn fields(&mut self) -> Vec<&mut TextField> {
        match self.active {
            Some(ScreenKind::Playlists) => vec![
                &mut self.playlists.name,
                &mut self.playlists.new_name,
                &mut self.playlists.search,
                &mut self.playlists.url,
            ],
            Some(ScreenKind::History) => vec![&mut self.history.search],
            Some(ScreenKind::Controls) => vec![&mut self.controls.url],
            _ => vec![],
        }
    }
    /// Clipboard handling is separate from text editing so tests can supply text without an OS clipboard.
    pub fn paste(&mut self, text: &str) -> bool {
        let focus = self.input.focus;
        for f in self.fields() {
            if focus == Some(f.id()) {
                f.insert_text(text);
                return true;
            }
        }
        false
    }
    pub fn input(&mut self, event: &InputEvent) -> (bool, Vec<ScreenAction>) {
        if self.active.is_none() {
            return (false, vec![]);
        }
        if matches!(event, InputEvent::Cancel) {
            self.cancel_input();
            return (false, vec![]);
        }
        if matches!(
            event,
            InputEvent::Key {
                key: Key::Escape,
                ..
            }
        ) {
            self.close();
            return (false, vec![]);
        }
        // Layout/IDs must exist before accepting input after navigation.
        if self.dirty {
            return (true, vec![]);
        }
        if let InputEvent::Scroll { at, delta, .. } = event
            && self.active == Some(ScreenKind::Settings)
            && self.content.contains(*at)
        {
            self.scroll = (self.scroll - delta * 24.).clamp(0., self.max_scroll);
            return (true, vec![]);
        }
        let mut widgets: Vec<&mut dyn Widget> = self
            .buttons
            .iter_mut()
            .map(|b| &mut b.button as &mut dyn Widget)
            .collect();
        match self.active {
            Some(ScreenKind::Playlists) => {
                widgets.extend([
                    &mut self.playlists.name as &mut dyn Widget,
                    &mut self.playlists.new_name,
                    &mut self.playlists.search,
                    &mut self.playlists.url,
                    &mut self.playlists.names,
                    &mut self.playlists.tracks,
                ]);
            }
            Some(ScreenKind::History) => {
                widgets.extend([
                    &mut self.history.search as &mut dyn Widget,
                    &mut self.history.rows,
                ]);
            }
            Some(ScreenKind::Controls) => {
                widgets.extend([
                    &mut self.controls.url as &mut dyn Widget,
                    &mut self.controls.rows,
                ]);
            }
            _ => {}
        }
        if let InputEvent::Key {
            key: Key::Tab,
            mods,
        } = event
        {
            let mut ids = vec![];
            for w in widgets
                .iter()
                .filter(|w| w.rect().w > 0. && w.rect().h > 0.)
            {
                w.focusables(&mut ids);
            }
            if ids.is_empty() {
                return (true, vec![]);
            }
            let i = self
                .input
                .focus
                .and_then(|id| ids.iter().position(|v| *v == id));
            let next = if mods.shift {
                i.map_or(ids.len() - 1, |i| (i + ids.len() - 1) % ids.len())
            } else {
                i.map_or(0, |i| (i + 1) % ids.len())
            };
            self.input.focus = Some(ids[next]);
            if self.active == Some(ScreenKind::Settings) {
                let r = widgets.iter().find(|w| w.id() == ids[next]).unwrap().rect();
                if r.y < self.content.y {
                    self.scroll = (self.scroll + r.y - self.content.y).max(0.);
                } else if r.bottom() > self.content.bottom() {
                    self.scroll =
                        (self.scroll + r.bottom() - self.content.bottom()).min(self.max_scroll);
                }
            }
            return (true, vec![]);
        }
        widgets.retain(|w| w.rect().w > 0. && w.rect().h > 0.);
        let mut actions = vec![];
        let mut ctx = EventContext {
            input: &mut self.input,
            actions: &mut actions,
        };
        // A captured drag/release belongs to its original widget.
        let capture = ctx.input.capture;
        let mut used = false;
        if let Some(id) = capture
            && let Some(w) = widgets.iter_mut().find(|w| w.id() == id)
        {
            used = w.event(event, &mut ctx);
        }
        if !used {
            for w in &mut widgets {
                if let Some(at) = event.position()
                    && (w.rect().y >= self.content.y && !self.content.contains(at))
                {
                    continue;
                }
                if w.event(event, &mut ctx) {
                    break;
                }
            }
        }
        let mut out = vec![];
        for a in actions {
            match a {
                UiAction::Click(id) => {
                    if let Some(b) = self.buttons.iter().find(|b| b.button.id() == id) {
                        if b.action == ScreenAction::DeletePlaylist {
                            if self
                                .playlists
                                .delete_armed
                                .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3))
                            {
                                out.push(b.action.clone());
                                self.playlists.delete_armed = None;
                            } else {
                                self.playlists.delete_armed = Some(std::time::Instant::now());
                                self.notice =
                                    "Click Delete again within 3 seconds to remove this playlist"
                                        .into();
                            }
                        } else {
                            out.push(b.action.clone());
                        }
                    }
                }
                UiAction::Select(id, i) if id == self.playlists.names.id() => {
                    self.select_playlist(i);
                }
                UiAction::Reorder(id, from, to) if id == self.playlists.tracks.id() => {
                    out.push(ScreenAction::Reorder(from, to))
                }
                UiAction::Activate(id, i) if id == self.playlists.tracks.id() => {
                    self.playlists.tracks.selected = Some(i);
                    out.push(ScreenAction::PlayEntry(false));
                }
                UiAction::Remove(id, i) if id == self.playlists.tracks.id() => {
                    self.playlists.tracks.selected = Some(i);
                    out.push(ScreenAction::RemoveEntry);
                }
                UiAction::Submit(id, _) if id == self.playlists.new_name.id() => {
                    out.push(ScreenAction::Create)
                }
                UiAction::Submit(id, _) if id == self.playlists.name.id() => {
                    out.push(ScreenAction::Rename)
                }
                UiAction::Submit(id, _) if id == self.playlists.url.id() => {
                    out.push(ScreenAction::AddUrl)
                }
                UiAction::Submit(id, _) if id == self.controls.url.id() => {
                    out.push(ScreenAction::OpenUrl(true))
                }
                _ => {}
            }
        }
        let selected = self
            .controls
            .rows
            .selected
            .and_then(|i| self.controls.ids.get(i))
            .copied();
        out.retain_mut(|a| match a {
            ScreenAction::WindowReveal(id)
            | ScreenAction::WindowHide(id)
            | ScreenAction::WindowClose(id)
            | ScreenAction::PlayerToggle(id)
            | ScreenAction::PlayerNext(id) => {
                if let Some(s) = selected {
                    *id = s;
                    true
                } else {
                    self.notice = "Select a player".into();
                    false
                }
            }
            _ => true,
        });
        (true, out)
    }
}

fn setting<'a>(s: &'a Value, key: &str) -> &'a Value {
    if !s[key].is_null() {
        return &s[key];
    }
    let alias = match key {
        "replaceChatLinks" => "chat.replace_links",
        "chatHoverPreviews" => "chat.hover_previews",
        "preferSystemBinaries" => "tools.prefer_system_binaries",
        "autoDownloadTools" => "tools.auto_download",
        "backgroundMode" => "playback.background_mode",
        "maxConcurrentVideos" => "performance.max_concurrent_videos",
        _ => return &s[key],
    };
    let value = &s["values"][format!("liasmediaplayer:{alias}")];
    if key == "chatHoverPreviews" && value.is_null() {
        &s["values"]["liasmediaplayer:chat.hover_preview"]
    } else {
        value
    }
}
fn wrap(c: &dyn Canvas, text: &str, width: f32) -> Vec<String> {
    let mut lines = vec![];
    let mut line = String::new();
    for ch in text.chars() {
        if !line.is_empty() && c.text_width(&format!("{line}{ch}")) > width {
            lines.push(std::mem::take(&mut line));
        }
        line.push(ch);
    }
    lines.push(line);
    lines
}
