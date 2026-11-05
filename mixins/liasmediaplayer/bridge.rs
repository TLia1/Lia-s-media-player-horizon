// Bridge between Horizon's `Game` hook points and the `lias_media_player` crate.
//
// Injected as `pub(crate) mod liasmediaplayer_bridge` inside `crates/horizon-client/src/game.rs`
// by `horizon-mixins` (`[[module]]` in `horizon-mixin.toml`). Keeping all Horizon-specific types
// (`Game`, `HudBatch`, `Corners`, `NativeMod`) in this small module means `lias_media_player`
// compiles once on its own and tests with standard `cargo test`.

use std::time::Duration;

/// Register Lia's Media Player and its rebindable keys in `Options > Mods` (`native_mods`).
pub(super) fn on_mods(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    let keys = lias_media_player::runtime::MOD_KEYS
        .iter()
        .map(|[name, label, default_key]| {
            [
                (*name).to_owned(),
                (*label).to_owned(),
                (*default_key).to_owned(),
            ]
        })
        .collect();
    mods.push(crate::game::native_hooks::NativeMod {
        id: lias_media_player::ID.into(),
        title: lias_media_player::title(),
        description: lias_media_player::DESCRIPTION.into(),
        keys,
    });
}

/// Populate the buttons on `Options > Mods > Lia's Media Player` (`native_settings`).
pub(super) fn on_settings(
    game: &mut crate::game::Game<'_>,
    mod_id: &str,
    buttons: &mut Vec<String>,
) {
    if mod_id != lias_media_player::ID {
        return;
    }
    let mut list = lias_media_player::runtime::with_runtime(|rt| {
        rt.configure(&game.native_data_dir(lias_media_player::ID), &game.native_instance_dir().join(lias_media_player::ID));
        rt.settings_buttons(
            || {
                game.native_setting(
                    lias_media_player::ID,
                    lias_media_player::runtime::KEY_HUD_BADGE,
                )
            },
            || {
                game.native_setting(
                    lias_media_player::ID,
                    lias_media_player::runtime::KEY_REPLACE_CHAT_LINKS,
                )
            },
        )
    });
    if game.title.is_some() || game.player.is_none() {
        list[2] = "Open Full Media Player Settings... (enter a world first)".into();
    } else if !game.native_on(lias_media_player::ID) {
        list[2] = "Open Full Media Player Settings... (enable the mod first)".into();
    }
    buttons.extend(list);
}

/// Handle a button click on `Options > Mods > Lia's Media Player` (`native_settings_click`).
pub(super) fn on_settings_click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    if mod_id != lias_media_player::ID {
        return;
    }
    match button {
        0 => {
            let next = lias_media_player::runtime::with_runtime(|rt| {
                let cur = rt.hud_badge_shown(|| {
                    game.native_setting(
                        lias_media_player::ID,
                        lias_media_player::runtime::KEY_HUD_BADGE,
                    )
                });
                let next = !cur;
                rt.set_hud_badge_shown(next);
                next
            });
            game.native_set_setting(
                lias_media_player::ID,
                lias_media_player::runtime::KEY_HUD_BADGE,
                if next { "true" } else { "false" },
            );
        }
        1 => {
            let next = lias_media_player::runtime::with_runtime(|rt| {
                let cur = rt.replace_chat_links(|| {
                    game.native_setting(
                        lias_media_player::ID,
                        lias_media_player::runtime::KEY_REPLACE_CHAT_LINKS,
                    )
                });
                let next = !cur;
                rt.set_replace_chat_links(next);
                next
            });
            game.native_set_setting(
                lias_media_player::ID,
                lias_media_player::runtime::KEY_REPLACE_CHAT_LINKS,
                if next { "true" } else { "false" },
            );
            game.ui.chat.rescale(&game.gui.atlas);
        }
        2 => {
            // API 359 opens native screens only over Playing. Close our invoking Options
            // page through Horizon's normal save/close path before opening the view.
            if game.title.is_some() || game.player.is_none() {
                game.native_chat("[Lia's Media Player] Enter a world to open full settings.");
                return;
            }
            if !game.native_on(lias_media_player::ID) {
                game.native_chat("[Lia's Media Player] Enable the mod to open full settings.");
                return;
            }
            if game.screen == super::Screen::Options {
                game.options_to_game = true;
                if game.handle_option_events(vec![horizon_client::options_ui::Event::Close]).is_err() { return; }
            }
            if game.native_mod_screen_open(lias_media_player::ID) {
                lias_media_player::runtime::with_runtime(|rt| rt.open_screen(lias_media_player::ui::screens::ScreenKind::Settings));
            }
        }
        _ => {}
    }
}

/// Register client-side commands (`/show`) in Horizon's command tree (`native_commands`).
pub(super) fn on_commands(game: &mut crate::game::Game<'_>, commands: &mut Vec<&'static str>) {
    if !game.native_on(lias_media_player::ID) {
        return;
    }
    for &cmd in lias_media_player::COMMANDS {
        if !commands.contains(&cmd) {
            commands.push(cmd);
        }
    }
}

/// Execute `/show <image|video|audio> <url> [newPlayer]` on the client (`native_command`).
pub(super) fn on_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    if *ran || !game.native_on(lias_media_player::ID) {
        return;
    }
    if let Some(url) = command.strip_prefix("show _chat ") {
        *ran = true;
        lias_media_player::runtime::with_runtime(|rt| {
            if let Some(action) = lias_media_player::core::chat::click_action(url, Default::default(), &rt.sources) { rt.chat_action(action); }
        });
        return;
    }
    let Some(parsed) = lias_media_player::command::parse(command) else {
        return;
    };
    *ran = true;
    match parsed {
        Ok(cmd) => {
            let root = game.native_data_dir(lias_media_player::ID);
            let java = game.native_instance_dir().join(lias_media_player::ID);
            let reply = lias_media_player::runtime::with_runtime(|rt| {
                rt.configure(&root, &java);
                rt.execute_show(cmd)
            });
            game.native_chat(&reply);
        }
        Err(err) => {
            game.native_chat(&format!("[Lia's Media Player] {err}"));
        }
    }
}

/// Client tick hook (`native_tick`).
pub(super) fn on_tick(game: &mut crate::game::Game<'_>) {
    if !game.native_window() || !game.native_on(lias_media_player::ID) {
        return;
    }
    lias_media_player::runtime::with_runtime(|rt| {
        if game.native_playing() {
            rt.configure(&game.native_data_dir(lias_media_player::ID), &game.native_instance_dir().join(lias_media_player::ID));
        }
        rt.on_tick();
    });
}

/// Per-frame hook (`native_frame`).
pub(super) fn on_frame(game: &mut crate::game::Game<'_>, elapsed: Duration) {
    if !game.native_window() {
        return;
    }
    let enabled = game.native_on(lias_media_player::ID);
    if horizon_client::chat::liasmediaplayer_chat_transform::set(enabled, on_chat_received) {
        game.ui.chat.rescale(&game.gui.atlas);
    }
    if !enabled {
        lias_media_player::runtime::with_runtime(|rt| rt.on_disabled());
        return;
    }
    let rewrap = lias_media_player::runtime::with_runtime(|rt| std::mem::take(&mut rt.chat_rewrap_requested));
    if rewrap { game.ui.chat.rescale(&game.gui.atlas); }
    lias_media_player::runtime::with_runtime(|rt| {
        rt.interactive = game.screen.id() != "playing";
        if game.screen.id() != "chat" || !game.focused { rt.chat.clear_hover(rt.previews.as_mut(), &mut rt.ui.atlas); }
        if game.screen.id() != "mod" && !rt.open_playlist_screen { rt.chat.picker = None; }
        if rt.close_screen_requested {
            rt.close_screen_requested = false;
            game.native_mod_screen_close(lias_media_player::ID);
        }
        if game.native_mod_screen() != Some(lias_media_player::ID) && !rt.open_playlist_screen { rt.close_views(); }
        if !game.focused { rt.screens.cancel_input(); }
        rt.on_frame(elapsed);
        if rt.open_playlist_screen {
            rt.open_playlist_screen = false;
            if game.screen.id() == "chat" { game.close_chat(super::chat::ChatExit::Interrupted); }
            if !game.native_mod_screen_open(lias_media_player::ID) { rt.chat.notify("Close the current screen to open playlists"); }
        }
        rt.ui.observe_screen(game.screen.id(), game.focused);
        rt.ui
            .atlas
            .flush(|w, h, rgba| game.native_mod_image(w, h, rgba));
    });
}

/// Draw a bounded, live playback summary in the top-right corner (`native_hud`).
pub(super) fn on_hud(
    game: &mut crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    corners: &mut horizon_client::extras_hud::Corners,
) {
    use horizon_client::extras::Corner;
    if game.native_on(lias_media_player::ID) && game.native_playing() {
        draw_windows(game, batch, false, None);
    }
    if !game.native_on(lias_media_player::ID) || !game.native_corner_shown(Corner::TopRight) || game.native_mod_screen() == Some(lias_media_player::ID) {
        return;
    }
    let badge = lias_media_player::runtime::with_runtime(|rt| {
        let shown = rt.hud_badge_shown(|| {
            game.native_setting(
                lias_media_player::ID,
                lias_media_player::runtime::KEY_HUD_BADGE,
            )
        });
        (shown && rt.hud_active()).then(|| rt.badge())
    });
    let Some(badge) = badge else {
        return;
    };

    // Fit to measured GUI pixels, including narrow windows and large GUI scales.
    let max_width = (batch.width - 16.0).clamp(0.0, 280.0);
    let fit = |text: &str| {
        let mut text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if game.native_text_width(&text) > max_width - 12.0 {
            while !text.is_empty() && game.native_text_width(&format!("{text}…")) > max_width - 12.0 { text.pop(); }
            if max_width >= 18.0 { text.push('…'); }
        }
        text
    };
    let header = fit(&badge.header);
    let status = fit(&badge.status);
    let w = (game.native_text_width(&header).max(game.native_text_width(&status)) + 12.0).min(max_width);
    let h = 29.0;
    let (x, y) = corners.place(Corner::TopRight, w, h);
    let start = batch.vertices.len();
    game.native_fill(batch, [x, y, w, h], 0xE8141821);
    game.native_fill(batch, [x, y, 2.0, h], 0xFF69BDFF);
    game.native_text(batch, &header, [x + 6.0, y + 4.0], 0xFFFFFF, false);
    game.native_text(batch, &status, [x + 6.0, y + 15.0], 0xBAC5D6, false);
    if let Some(progress) = badge.progress {
        game.native_fill(batch, [x, y + h - 2.0, w, 2.0], 0xFF34435B);
        game.native_fill(batch, [x, y + h - 2.0, w * progress as f32 / 1000.0, 2.0], 0xFF69BDFF);
    }
    batch.clip_since(start, x.max(0.0), y.max(0.0), (x + w).min(batch.width), (y + h).min(batch.height));
}

/// Gameplay key hook (`native_key`).
pub(super) fn on_gameplay_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    if !pressed || !game.native_playing() || !game.native_on(lias_media_player::ID) {
        return;
    }
    for (name, default, command) in [
        ("play_pause", "key.keyboard.k", lias_media_player::core::player::PlayerCommand::Toggle),
        ("next", "key.keyboard.n", lias_media_player::core::player::PlayerCommand::Next),
    ] {
        if key == game.native_mod_key(lias_media_player::ID, name, default) {
            lias_media_player::runtime::with_runtime(|rt| {
                if let Some(playback) = &rt.playback { playback.command_front(&rt.ui.windows, command); }
            });
            return;
        }
    }
    for (name, default, screen) in [
        ("controls", "key.keyboard.m", lias_media_player::ui::screens::ScreenKind::Controls),
        ("playlists", "key.keyboard.p", lias_media_player::ui::screens::ScreenKind::Playlists),
        ("history", "key.keyboard.h", lias_media_player::ui::screens::ScreenKind::History),
    ] {
        if key == game.native_mod_key(lias_media_player::ID, name, default) {
            if game.native_mod_screen_open(lias_media_player::ID) {
                lias_media_player::runtime::with_runtime(|rt| {
                    rt.configure(&game.native_data_dir(lias_media_player::ID), &game.native_instance_dir().join(lias_media_player::ID));
                    if screen == lias_media_player::ui::screens::ScreenKind::Controls { rt.open_controls(); }
                    else { rt.open_screen(screen); }
                });
            }
            return;
        }
    }
}

/// Client level unload hook (`native_client_level_unload`).
pub(super) fn on_world_unload(game: &mut crate::game::Game<'_>, _dimension: &str) {
    if !game.native_window() {
        return;
    }
    lias_media_player::runtime::with_runtime(|rt| rt.on_world_unload());
}

// Rendering stays read-only: screen_render_post has &Game, unlike the mutable frame hook.
struct GameHud<'a, 'game> {
    game: &'a crate::game::Game<'game>,
    batch: &'a mut horizon_client::hud::HudBatch,
}
impl lias_media_player::platform::horizon_impl::HudBackend for GameHud<'_, '_> {
    fn size(&self) -> [f32; 2] {
        [self.batch.width, self.batch.height]
    }
    fn fill(&mut self, rect: lias_media_player::platform::Rect, argb: u32) {
        self.game.native_fill(self.batch, rect.array(), argb);
    }
    fn sprite(&mut self, source: [f32; 4], rect: lias_media_player::platform::Rect) {
        self.game.native_sprite(
            self.batch,
            lias_media_player::platform::horizon_impl::ATLAS_SPRITE,
            source,
            rect.array(),
        );
    }
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool) {
        let [a, r, g, b] = argb.to_be_bytes();
        self.batch.text_styled(
            &self.game.gui.atlas,
            &horizon_client::text::Text::literal(text),
            at[0],
            at[1],
            horizon_client::hud::rgba(r, g, b, a),
            shadow,
        );
    }
    fn text_width(&self, text: &str) -> f32 {
        self.game.native_text_width(text)
    }
    fn vertex_count(&self) -> usize {
        self.batch.vertices.len()
    }
    fn clip_since(&mut self, start: usize, rect: lias_media_player::platform::Rect) {
        self.batch
            .clip_since(start, rect.x, rect.y, rect.right(), rect.bottom());
    }
}
fn draw_windows(
    game: &crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    interactive: bool,
    mouse: Option<[f32; 2]>,
) {
    lias_media_player::runtime::with_runtime(|rt| {
        let ui = &mut rt.ui;
        let mut canvas = lias_media_player::platform::horizon_impl::HorizonCanvas::new(
            GameHud { game, batch },
            &ui.atlas,
        );
        if interactive && game.native_mod_screen() == Some(lias_media_player::ID) {
            let empty = serde_json::json!({});
            let settings = rt.stores.as_ref().map_or(&empty, |s| s.settings.get());
            rt.screens.draw(&mut canvas, &ui.windows.theme, mouse, settings);
            rt.chat.draw_picker(&mut canvas);
        }
        if !interactive || rt.screens.active.is_none() || rt.screens.active == Some(lias_media_player::ui::screens::ScreenKind::Controls) {
            ui.windows.draw(&mut canvas, interactive, mouse);
        }
        if interactive && game.native_mod_screen() == Some(lias_media_player::ID) { rt.chat.draw_notice(&mut canvas); }
        if !interactive {
            if let Some(playback) = &rt.playback { playback.draw_banner(&mut canvas); }
        }
    });
}
pub(super) fn on_screen_render(
    game: &crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
) {
    if game.native_on(lias_media_player::ID) {
        if screen.id == "chat" {
            let url = chat_url(game,screen);
            lias_media_player::runtime::with_runtime(|rt| {
                let covered = rt.ui.windows.windows().iter().any(|w| !w.hidden && (w.theatre || w.rect.contains(screen.mouse)));
                let hidden = rt.ui.windows.windows().iter().filter(|w|w.hidden).count();
                let chip = lias_media_player::ui::chat_overlay::ChatOverlay::chip_click([screen.rect[2],screen.rect[3]],hidden,screen.mouse).is_some();
                let media = if !game.focused || covered || chip || !rt.chat_hover_previews { None } else { url.as_deref().and_then(|u|rt.sources.identify(u)) };
                rt.chat.hover(media,rt.previews.as_mut(),&mut rt.ui.atlas);
                let mut canvas=lias_media_player::platform::horizon_impl::HorizonCanvas::new(GameHud { game, batch },&rt.ui.atlas);
                rt.chat.draw_chat(&mut canvas,screen.mouse,hidden,rt.previews.as_ref());
            });
        }
        draw_windows(game, batch, true, Some(screen.mouse));
    }
}
pub(super) fn on_input(
    game: &crate::game::Game<'_>,
    event: lias_media_player::platform::InputEvent,
    canceled: &mut bool,
) {
    if *canceled || !game.native_on(lias_media_player::ID) {
        return;
    }
    *canceled = lias_media_player::runtime::with_runtime(|rt| {
        if game.native_mod_screen() != Some(lias_media_player::ID) { rt.close_views(); rt.chat.picker = None; }
        if !game.focused { rt.screens.cancel_input(); return false; }
        rt.input(&event)
    });
}
pub(super) fn on_resources_reloaded() {
    lias_media_player::runtime::with_runtime(|rt| rt.ui.atlas.invalidate());
}

/// Pinned API-359 equivalent of client_chat_received: transform only the display copy.
pub(super) fn on_chat_received(
    mut message: horizon_client::text::Text,
) -> horizon_client::text::Text {
    use horizon_client::text::{ClickEvent, Span, Text};
    use lias_media_player::core::chat::{ChatSpan, CHAT_COMMAND, badge_color, rewrite};
    if !lias_media_player::core::chat::has_http(message.spans.iter().map(|s| s.text.as_str())) && !message.spans.iter().any(|s| matches!(s.style.click, Some(ClickEvent::OpenUrl(_)))) {
        return message;
    }
    lias_media_player::runtime::with_runtime(|rt| {
        let replace = rt.replace_chat_links(|| None);
        let spans: Vec<_> = message.spans.iter().map(|s| ChatSpan {
            text: s.text.clone(), style: s.style.clone(),
            url: match &s.style.click { Some(ClickEvent::OpenUrl(url)) => Some(url.clone()), _ => None },
            protected: s.style.inline_object.is_some() || matches!(&s.style.click, Some(click) if !matches!(click, ClickEvent::OpenUrl(_))),
        }).collect();
        message.spans = rewrite(&spans, &rt.sources, replace).into_iter().map(|span| {
            let mut style = span.style;
            if let Some(media) = span.media {
                style.color = Some(badge_color(&media));
                style.underlined = Some(false);
                style.click = Some(ClickEvent::RunCommand(format!("{CHAT_COMMAND}{}", media.url)));
                style.insertion = Some(media.url);
                // The overlay supplies its own tooltip; suppress inherited hover events.
                style.hover = None; style.hover_item = None; style.hover_entity = None;
            }
            Span { text: span.text, style }
        }).collect();
    });
    // Rebuild a display-only Text, dropping the original serialized component cache.
    let mut display = Text::default();
    display.spans = message.spans;
    display
}
fn chat_url(game: &crate::game::Game<'_>, screen: &crate::game::native_hooks::NativeScreenView<'_>) -> Option<String> {
    use horizon_client::text::ClickEvent;
    if screen.id != "chat" { return None; }
    let style = game.ui.chat.style_at(&game.gui.atlas.font,screen.rect[3] as i32,screen.mouse[0] as f64,screen.mouse[1] as f64)?;
    match style.click {
        Some(ClickEvent::OpenUrl(url)) => Some(url),
        Some(ClickEvent::RunCommand(cmd)) => cmd.strip_prefix(lias_media_player::core::chat::CHAT_COMMAND).map(str::to_owned),
        _ => None,
    }
}
pub(super) fn on_chat_mouse(
    game: &crate::game::Game<'_>, screen: &crate::game::native_hooks::NativeScreenView<'_>, button:i32, modifiers:u32, canceled:&mut bool,
) {
    if *canceled || button != 0 || screen.id != "chat" || !game.native_on(lias_media_player::ID) { return; }
    let url=chat_url(game,screen);
    *canceled=lias_media_player::runtime::with_runtime(|rt| {
        let hidden=rt.ui.windows.windows().iter().filter(|w|w.hidden).count();
        let action=lias_media_player::ui::chat_overlay::ChatOverlay::chip_click([screen.rect[2],screen.rect[3]],hidden,screen.mouse)
            .or_else(||url.as_deref().and_then(|u|lias_media_player::core::chat::click_action(u,lias_media_player::platform::Modifiers::from_bits(modifiers),&rt.sources)));
        if let Some(action)=action { rt.chat_action(action); true } else { false }
    });
}
