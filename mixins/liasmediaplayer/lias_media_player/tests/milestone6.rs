use lias_media_player::{
    Runtime,
    core::{
        library,
        sources::SourceRegistry,
        tools::{ToolManager, ToolTask},
    },
    platform::*,
    ui::{screens::*, *},
};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static ID: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "lmp-m6-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn runtime(temp: &Temp) -> Runtime {
    fs::write(
        temp.0.join("settings.json"),
        br#"{"autoDownloadTools":false,"preferSystemBinaries":false}"#,
    )
    .unwrap();
    let mut rt = Runtime::new();
    rt.configure(&temp.0, &temp.0);
    rt
}
#[derive(Default)]
struct CanvasLog {
    size: [f32; 2],
    text: Vec<(String, [f32; 2])>,
    rects: Vec<Rect>,
    ops: Vec<serde_json::Value>,
}
impl Canvas for CanvasLog {
    fn size(&self) -> [f32; 2] {
        self.size
    }
    fn fill(&mut self, r: Rect, color: u32) {
        self.ops.push(json!(["fill", r.array(), color]));
        assert!(r.w >= 0. && r.h >= 0.);
        self.rects.push(r);
    }
    fn texture(&mut self, _: TextureId, _: Rect, _: [f32; 4]) {}
    fn text(&mut self, s: &str, p: [f32; 2], color: u32, _: bool) {
        self.ops.push(json!(["text", s, p, color]));
        self.text.push((s.into(), p));
    }
    fn text_width(&self, s: &str) -> f32 {
        s.chars().count() as f32 * 6.
    }
    fn push_clip(&mut self, r: Rect) {
        self.ops.push(json!(["clip", r.array()]));
    }
    fn pop_clip(&mut self) {
        self.ops.push(json!(["pop"]));
    }
}
fn draw(rt: &mut Runtime, size: [f32; 2]) -> CanvasLog {
    rt.sync_screens();
    let mut c = CanvasLog {
        size,
        ..Default::default()
    };
    let settings = rt
        .stores
        .as_ref()
        .map(|s| s.settings.get().clone())
        .unwrap_or(json!({}));
    rt.screens
        .draw(&mut c, &rt.ui.windows.theme, None, &settings);
    if rt.screens.active == Some(ScreenKind::Controls) {
        rt.ui.windows.draw(&mut c, true, None);
    }
    c
}
fn down(at: [f32; 2]) -> InputEvent {
    InputEvent::Down {
        at,
        button: 0,
        mods: Modifiers::default(),
    }
}
fn up(at: [f32; 2]) -> InputEvent {
    InputEvent::Up { at, button: 0 }
}
fn wait(mut test: impl FnMut() -> bool) {
    let start = Instant::now();
    while !test() {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn library_edits_preserve_java_shapes_and_metadata_and_are_atomic_at_limit() {
    let mut lib = json!({"unknown":7,"playlists":[{"name":"Java","custom":true,"urls":["https://a.test/1.mp3",{"url":"https://a.test/2.mp3","extra":42}]}]});
    library::rename(&mut lib, 0, "Été").unwrap();
    library::reorder(&mut lib, 0, 1, 0).unwrap();
    assert_eq!(lib["playlists"][0]["urls"][0]["extra"], 42);
    assert_eq!(lib["playlists"][0]["custom"], true);
    library::append(&mut lib, 0, vec![json!("https://a.test/3.mp3")]).unwrap();
    library::remove(&mut lib, 0, 1).unwrap();
    assert_eq!(library::entries(&lib["playlists"][0]).len(), 2);
    let before = lib.clone();
    assert!(library::append(&mut lib, 0, vec![json!("https://a.test/a.mp3"); 499]).is_err());
    assert_eq!(lib, before);
    assert!(library::append(&mut lib, 0, vec![json!("file:///tmp/a")]).is_err());
    assert_eq!(lib, before);
    let i = library::create(&mut lib, "Rust").unwrap();
    assert!(!lib["playlists"][i]["id"].as_str().unwrap().is_empty());
    library::delete(&mut lib, 0).unwrap();
    assert_eq!(library::name(&lib["playlists"][0]), "Rust");
    assert_eq!(lib["unknown"], 7);
}

#[test]
fn clipboard_playlist_preserves_m3u_titles_and_rejects_empty_or_oversized_lists() {
    let entries = library::clipboard_entries("#EXTM3U\n#EXTINF:-1,Été song\nhttps://a.test/a.mp3\n# a comment\nhttps://a.test/b.mp3\nfile:///private/audio.mp3").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(library::label(&entries[0]), "Été song");
    assert_eq!(library::label(&entries[1]), "b.mp3");
    assert!(library::clipboard_entries("#EXTM3U\nfile:///music.mp3").is_err());
    assert!(library::clipboard_entries(&"https://a.test/a.mp3\n".repeat(501)).is_err());
}
#[test]
fn history_search_and_source_filters_map_back_to_original_rows() {
    let sources = SourceRegistry::default();
    let lib = json!({"history":[{"url":"https://a.test/a.mp3","title":"Piano"},{"url":"https://youtu.be/abc","title":"ÉTÉ"},{"url":"https://a.test/b.mp3","title":"piano two"}]});
    assert_eq!(
        library::history_indices(&lib, "PIANO", None, &sources),
        [0, 2]
    );
    let youtube = sources.identify("https://youtu.be/abc").unwrap().source_id;
    assert_eq!(
        library::history_indices(&lib, "été", Some(&youtube), &sources),
        [1]
    );
    assert!(library::history_indices(&lib, "a.test", Some(&youtube), &sources).is_empty());
    assert!(
        library::media(
            &json!({"url":"https://a.test/opaque","kind":"AUDIO"}),
            &sources
        )
        .is_some()
    );
}
#[test]
fn list_drag_reorders_only_on_valid_release_and_cancel_discards_drag() {
    let mut list = ListView::new(vec!["a".into(), "b".into(), "c".into()]);
    list.reorderable = true;
    list.layout(Rect::new(0., 0., 100., 60.));
    let mut input = InputState::default();
    let mut actions = vec![];
    for e in [
        down([10., 8.]),
        InputEvent::Drag {
            at: [10., 48.],
            button: 0,
            mods: Modifiers::default(),
        },
        up([10., 48.]),
    ] {
        list.event(
            &e,
            &mut EventContext {
                input: &mut input,
                actions: &mut actions,
            },
        );
    }
    assert!(actions.contains(&UiAction::Reorder(list.id(), 0, 2)));
    assert_eq!(input.capture, None);
    actions.clear();
    for e in [
        down([10., 8.]),
        InputEvent::Drag {
            at: [10., 48.],
            button: 0,
            mods: Modifiers::default(),
        },
        InputEvent::Cancel,
        up([10., 48.]),
    ] {
        list.event(
            &e,
            &mut EventContext {
                input: &mut input,
                actions: &mut actions,
            },
        );
    }
    assert!(!actions.iter().any(|a| matches!(a, UiAction::Reorder(..))));
    list.reorderable = false;
    actions.clear();
    for e in [
        down([10., 8.]),
        InputEvent::Drag {
            at: [10., 48.],
            button: 0,
            mods: Modifiers::default(),
        },
        up([10., 48.]),
    ] {
        list.event(
            &e,
            &mut EventContext {
                input: &mut input,
                actions: &mut actions,
            },
        );
    }
    assert!(!actions.iter().any(|a| matches!(a, UiAction::Reorder(..))));
}
#[test]
fn screens_edit_reorder_and_persist_library_through_real_input() {
    let temp = Temp::new();
    let mut rt = runtime(&temp);
    rt.open_screen(ScreenKind::Playlists);
    rt.screens.playlists.new_name = TextField::new("Road trip");
    rt.screen_action(ScreenAction::Create);
    for url in [
        "https://a.test/1.mp3",
        "https://a.test/2.mp3",
        "https://a.test/3.mp3",
    ] {
        rt.screens.playlists.url = TextField::new(url);
        rt.screen_action(ScreenAction::AddUrl);
    }
    draw(&mut rt, [800., 480.]);
    let r = rt.screens.playlists.tracks.rect();
    for e in [
        down([r.x + 10., r.y + 8.]),
        InputEvent::Drag {
            at: [r.x + 10., r.y + 48.],
            button: 0,
            mods: Modifiers::default(),
        },
        up([r.x + 10., r.y + 48.]),
    ] {
        assert!(rt.input(&e));
    }
    let lib = rt.stores.as_ref().unwrap().library.get();
    assert_eq!(
        lib["playlists"][0]["entries"][2]["url"],
        "https://a.test/1.mp3"
    );
    rt.stores.as_ref().unwrap().flush().unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.0.join("library.json")).unwrap()).unwrap();
    assert_eq!(&saved, lib);
}

#[test]
fn playlist_filtered_rows_select_without_deleting_and_remove_the_correct_track() {
    let temp = Temp::new();
    let mut rt = runtime(&temp);
    rt.stores
        .as_mut()
        .unwrap()
        .library
        .set(json!({"playlists":[{"name":"Mix","entries":[
            {"url":"https://a.test/a.mp3","title":"Alpha"},
            {"url":"https://a.test/b.mp3","title":"Beta"},
            {"url":"https://a.test/c.mp3","title":"Gamma"}
        ]}]}))
        .unwrap();
    rt.open_screen(ScreenKind::Playlists);
    rt.screens.select_playlist(0);
    rt.screens.playlists.search = TextField::new("Beta");
    draw(&mut rt, [320., 240.]);
    let r = rt.screens.playlists.tracks.rect();
    rt.input(&down([r.x + 12., r.y + 8.]));
    rt.input(&up([r.x + 12., r.y + 8.]));
    assert_eq!(rt.screens.selected_entry(), Some(1));
    assert_eq!(
        library::entries(&library::lists(rt.stores.as_ref().unwrap().library.get())[0]).len(),
        3
    );
    rt.input(&down([r.right() - 8., r.y + 8.]));
    rt.input(&up([r.right() - 8., r.y + 8.]));
    let entries = library::entries(&library::lists(rt.stores.as_ref().unwrap().library.get())[0]);
    assert_eq!(
        entries.iter().map(library::label).collect::<Vec<_>>(),
        ["Alpha", "Gamma"]
    );
}

#[test]
fn enter_creates_a_named_playlist_and_delete_requires_a_second_click() {
    let temp = Temp::new();
    let mut rt = runtime(&temp);
    rt.open_screen(ScreenKind::Playlists);
    draw(&mut rt, [320., 240.]);
    let r = rt.screens.playlists.new_name.rect();
    rt.input(&down([r.x + 5., r.y + 5.]));
    rt.input(&up([r.x + 5., r.y + 5.]));
    assert!(rt.screens.paste("My music"));
    rt.input(&InputEvent::Key {
        key: Key::Enter,
        mods: Modifiers::default(),
    });
    assert_eq!(
        library::lists(rt.stores.as_ref().unwrap().library.get())[0]["name"],
        "My music"
    );
    let c = draw(&mut rt, [320., 240.]);
    let at = c.text.iter().find(|(s, _)| s == "Delete").unwrap().1;
    for i in 0..2 {
        rt.input(&down([at[0] + 2., at[1] + 2.]));
        rt.input(&up([at[0] + 2., at[1] + 2.]));
        assert_eq!(
            library::lists(rt.stores.as_ref().unwrap().library.get()).len(),
            1 - i
        );
    }
}
#[test]
fn full_views_route_navigation_typing_paste_and_escape_without_demo_windows() {
    let mut rt = Runtime::new();
    rt.open_controls();
    assert!(rt.ui.windows.windows().is_empty());
    let c = draw(&mut rt, [800., 480.]);
    let at = c.text.iter().find(|(s, _)| s == "History").unwrap().1;
    for e in [down([at[0] + 2., at[1] + 2.]), up([at[0] + 2., at[1] + 2.])] {
        assert!(rt.input(&e));
    }
    assert_eq!(rt.screens.active, Some(ScreenKind::History));
    draw(&mut rt, [800., 480.]);
    let field = rt.screens.history.search.rect();
    rt.input(&down([field.x + 4., field.y + 4.]));
    rt.input(&up([field.x + 4., field.y + 4.]));
    assert!(rt.screens.paste("Été\n猫"));
    assert_eq!(rt.screens.history.search.text(), "Été猫");
    rt.input(&InputEvent::Char('!'));
    assert_eq!(rt.screens.history.search.text(), "Été猫!");
    assert!(!rt.input(&InputEvent::Key {
        key: Key::Escape,
        mods: Modifiers::default()
    }));
    assert_eq!(rt.screens.active, None);
}
#[test]
fn settings_apply_live_and_survive_reload() {
    let temp = Temp::new();
    let mut rt = runtime(&temp);
    rt.open_screen(ScreenKind::Settings);
    for (key, value) in [
        ("theme", json!("liasmediaplayer:light")),
        ("replaceChatLinks", json!(false)),
        ("chatHoverPreviews", json!(false)),
        ("maxConcurrentVideos", json!(4)),
        ("backgroundMode", json!("keep_video")),
        ("hudBadge", json!(false)),
    ] {
        rt.screen_action(ScreenAction::Setting(key, value));
    }
    assert_eq!(rt.ui.windows.theme.id, "liasmediaplayer:light");
    assert!(!rt.chat_hover_previews);
    assert!(!rt.replace_chat_links(|| None));
    assert!(rt.chat_rewrap_requested);
    assert!(!rt.hud_badge_shown(|| None));
    assert_eq!(rt.playback.as_ref().unwrap().max_concurrent_videos, 4);
    rt.stores.as_ref().unwrap().flush().unwrap();
    drop(rt);
    let mut restored = Runtime::new();
    restored.configure(&temp.0, &temp.0);
    assert_eq!(restored.ui.windows.theme.id, "liasmediaplayer:light");
    assert!(!restored.chat_hover_previews);
    assert!(!restored.hud_badge_shown(|| None));
}
#[test]
fn history_remove_uses_filtered_row_without_floating_window_interception() {
    let temp = Temp::new();
    let mut rt = runtime(&temp);
    rt.stores.as_mut().unwrap().library.set(json!({"history":[{"url":"https://a.test/a.mp3","title":"First"},{"url":"https://a.test/b.mp3","title":"Second"}]})).unwrap();
    rt.open_screen(ScreenKind::History);
    rt.screens.history.search = TextField::new("Second");
    draw(&mut rt, [800., 480.]);
    let r = rt.screens.history.rows.rect();
    let mut window = FloatingWindow::new(
        "Cover",
        Rect::new(r.x, r.y, 220., 100.),
        Box::new(Button::new("Window action")),
    );
    window.min_size = [120., 80.];
    let id = window.id;
    rt.ui.windows.open(window);
    draw(&mut rt, [800., 480.]);
    assert!(rt.input(&down([r.x + 12., r.y + 8.])));
    assert_eq!(rt.screens.selected_history(), Some(1));
    rt.input(&up([r.x + 12., r.y + 8.]));
    rt.ui.windows.close(id);
    draw(&mut rt, [800., 480.]);
    rt.input(&down([r.x + 12., r.y + 8.]));
    rt.input(&up([r.x + 12., r.y + 8.]));
    assert_eq!(rt.screens.selected_history(), Some(1));
    rt.screen_action(ScreenAction::HistoryRemove);
    assert_eq!(
        rt.stores.as_ref().unwrap().library.get()["history"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        rt.stores.as_ref().unwrap().library.get()["history"][0]["title"],
        "First"
    );
}
#[test]
fn settings_and_views_render_finite_layouts_at_small_and_large_gui_sizes() {
    let mut rt = Runtime::new();
    for size in [[320., 240.], [854., 480.], [1920., 1080.]] {
        for kind in [
            ScreenKind::Controls,
            ScreenKind::Playlists,
            ScreenKind::History,
            ScreenKind::Settings,
        ] {
            rt.open_screen(kind);
            let c = draw(&mut rt, size);
            assert!(!c.text.is_empty());
            assert!(
                c.rects
                    .iter()
                    .all(|r| r.array().iter().all(|v| v.is_finite()))
            );
        }
    }
}
#[test]
fn tool_status_never_downloads_and_reports_missing_tools() {
    let temp = Temp::new();
    let mut tools = ToolManager::new(Some(&temp.0));
    tools.policy.prefer_system = false;
    let task = ToolTask::start(tools, None);
    let mut result = None;
    wait(|| {
        result = task.poll();
        result.is_some()
    });
    let status = result.unwrap();
    assert!(status.contains("yt-dlp: missing"));
    assert!(status.contains("ffmpeg: missing"));
    assert!(!temp.0.join("bin").exists());
}
#[cfg(unix)]
fn fake_collection_tool(temp: &Temp, delay: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(temp.0.join("bin")).unwrap();
    let path = temp.0.join("bin/yt-dlp");
    fs::write(&path,format!("#!/bin/sh\nif [ \"$1\" = '--version' ]; then echo 2026.11.05; else {delay}\necho '{{\"title\":\"Imported\",\"entries\":[{{\"url\":\"https://youtu.be/a\",\"title\":\"A\"}},{{\"url\":\"https://youtu.be/b\",\"title\":\"B\"}}]}}'; fi\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
#[cfg(unix)]
#[test]
fn collection_import_is_async_persistent_and_bound_to_target_playlist() {
    let temp = Temp::new();
    fake_collection_tool(&temp, "sleep 0.1;");
    let mut rt = runtime(&temp);
    rt.open_screen(ScreenKind::Playlists);
    rt.screens.playlists.new_name = TextField::new("Target");
    rt.screen_action(ScreenAction::Create);
    rt.screens.playlists.url = TextField::new("https://www.youtube.com/playlist?list=example");
    let start = Instant::now();
    rt.screen_action(ScreenAction::Import);
    assert!(start.elapsed() < Duration::from_millis(80));
    assert!(rt.screens.playlists.importing);
    rt.screens.playlists.new_name = TextField::new("Other");
    rt.screen_action(ScreenAction::Create);
    wait(|| {
        rt.poll_screen_jobs();
        !rt.screens.playlists.importing
    });
    let lib = rt.stores.as_ref().unwrap().library.get();
    assert_eq!(library::entries(&lib["playlists"][0]).len(), 2);
    assert!(library::entries(&lib["playlists"][1]).is_empty());
}
#[cfg(unix)]
#[test]
fn cancelled_and_deleted_imports_do_not_write_to_another_playlist() {
    for cancel in [false, true] {
        let temp = Temp::new();
        fake_collection_tool(&temp, "sleep 0.1;");
        let mut rt = runtime(&temp);
        rt.open_screen(ScreenKind::Playlists);
        rt.screens.playlists.new_name = TextField::new("Target");
        rt.screen_action(ScreenAction::Create);
        rt.screens.playlists.url = TextField::new("https://www.youtube.com/playlist?list=example");
        rt.screen_action(ScreenAction::Import);
        if cancel {
            rt.screen_action(ScreenAction::CancelImport);
        } else {
            rt.screen_action(ScreenAction::DeletePlaylist);
        }
        rt.screens.playlists.new_name = TextField::new("Other");
        rt.screen_action(ScreenAction::Create);
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(300) {
            rt.poll_screen_jobs();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            library::lists(rt.stores.as_ref().unwrap().library.get())
                .iter()
                .all(|p| library::entries(p).is_empty())
        );
    }
}

/// Optional draw-command artifacts for `tools/render_screens.py`; no OS window or network.
#[test]
fn screen_preview_artifacts() {
    let Ok(output) = std::env::var("LMP_SCREEN_PREVIEW_DIR") else {
        return;
    };
    fs::create_dir_all(&output).unwrap();
    let temp = Temp::new();
    let mut rt = runtime(&temp);
    rt.stores.as_mut().unwrap().library.set(json!({
        "playlists":[{"name":"Evening mix","urls":[{"url":"https://a.test/piano.mp3","title":"Quiet piano"},{"url":"https://youtu.be/abc","title":"Live concert"},{"url":"https://a.test/jazz.mp3","title":"Night jazz"}]},{"name":"Bookmarks","entries":[]}],
        "history":[{"url":"https://a.test/piano.mp3","title":"Quiet piano"},{"url":"https://youtu.be/abc","title":"Live concert"}]
    })).unwrap();
    for size in [[320., 240.], [854., 480.]] {
        for kind in [
            ScreenKind::Controls,
            ScreenKind::Playlists,
            ScreenKind::History,
            ScreenKind::Settings,
        ] {
            rt.open_screen(kind);
            if kind == ScreenKind::Playlists {
                rt.screens.select_playlist(0);
            }
            if kind == ScreenKind::Settings {
                rt.screen_action(ScreenAction::Tab(SettingsTab::Tools));
                rt.screens.settings.tool_status="yt-dlp: /home/player/horizon-mods/.horizon-mod-data/liasmediaplayer/bin/yt-dlp\nffmpeg: /usr/bin/ffmpeg".into();
            }
            let c = draw(&mut rt, size);
            let path = PathBuf::from(&output).join(format!("{kind:?}-{}.json", size[0]));
            fs::write(
                path,
                serde_json::to_vec(&json!({"size":size,"ops":c.ops})).unwrap(),
            )
            .unwrap();
        }
    }
}
#[test]
fn cancelled_redownload_keeps_the_previous_executable() {
    use lias_media_player::core::{process::Cancellation, tools::Tool};
    let temp = Temp::new();
    fs::create_dir_all(temp.0.join("bin")).unwrap();
    let path = temp.0.join("bin").join(Tool::YtDlp.name());
    fs::write(&path, b"existing binary").unwrap();
    let tools = ToolManager::new(Some(&temp.0));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(tools.redownload(Tool::YtDlp, &cancel).is_err());
    assert_eq!(fs::read(path).unwrap(), b"existing binary");
}

#[cfg(unix)]
#[test]
fn show_playlist_expands_before_creating_an_audio_player() {
    use lias_media_player::command::{ShowCommand, ShowKind};
    let temp = Temp::new();
    fake_collection_tool(&temp, "sleep 0.1;");
    let mut rt = runtime(&temp);
    let cmd = ShowCommand {
        kind: ShowKind::Audio,
        url: "https://youtube.com/watch?v=a&list=test".into(),
        new_player: true,
    };
    assert!(rt.execute_show(cmd.clone()).contains("Loading playlist"));
    assert!(rt.ui.windows.windows().is_empty());
    assert!(rt.hud_active());
    assert!(rt.badge().header.contains("Loading playlist"));
    assert!(rt.execute_show(cmd).contains("already loading"));
    wait(|| {
        rt.on_frame(Duration::from_millis(10));
        !rt.ui.windows.windows().is_empty()
    });
    let id = rt.ui.windows.windows()[0].id;
    assert!(rt.playback.as_ref().unwrap().get(id).unwrap().audio_only);
    // No collection URL is passed into the single-item resolver.
    wait(|| {
        let s = rt
            .playback
            .as_ref()
            .unwrap()
            .get(id)
            .unwrap()
            .player
            .control
            .snapshot();
        s.source_url == "https://youtu.be/a" && s.queue == vec!["https://youtu.be/b"]
    });
    rt.on_world_unload();
    assert!(!rt.hud_active());
}

#[cfg(unix)]
#[test]
fn add_url_imports_playlists_and_preserves_track_titles() {
    let temp = Temp::new();
    fake_collection_tool(&temp, "");
    let mut rt = runtime(&temp);
    rt.open_screen(ScreenKind::Playlists);
    rt.screens.playlists.new_name = TextField::new("Saved mix");
    rt.screen_action(ScreenAction::Create);
    rt.screens.playlists.url = TextField::new("https://youtu.be/a?list=test");
    rt.screen_action(ScreenAction::AddUrl);
    assert!(rt.screens.playlists.importing);
    wait(|| {
        rt.poll_screen_jobs();
        !rt.screens.playlists.importing
    });
    let entries = library::entries(&rt.stores.as_ref().unwrap().library.get()["playlists"][0]);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["title"], "A");
}

#[test]
fn diagnostics_wrap_long_unicode_urls_and_scroll_within_the_panel() {
    let mut c = CanvasLog {
        size: [320., 240.],
        ..Default::default()
    };
    let message = format!(
        "yt-dlp: ERROR\nhttps://example.test/{}\nlast diagnostic",
        "é".repeat(1500)
    );
    let lines = text::wrap(&c, &message, 120.);
    assert!(lines.iter().all(|line| c.text_width(line) <= 120.));
    let area = Rect::new(10., 10., 180., 130.);
    video_window::ErrorPanel::draw_scrolled(&mut c, area, &message, 10000);
    assert!(
        c.text
            .iter()
            .any(|(line, _)| line.contains("last diagnostic"))
    );
    for (text, [x, y]) in &c.text {
        assert!(
            *x >= area.x
                && *y >= area.y
                && *x + c.text_width(text) <= area.right()
                && *y + 9. <= area.bottom(),
            "out of bounds: {text}"
        );
    }
}
