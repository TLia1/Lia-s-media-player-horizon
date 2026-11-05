// Catalog hook adapter; the crate owns all UI behavior.
fn liasmediaplayer_screen_mouse_scrolled_pre(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    scroll: [f64; 2],
    modifiers: u32,
    canceled: &mut bool,
) {
    use lias_media_player::platform::InputEvent;
    use lias_media_player::platform::Modifiers;
    crate::game::liasmediaplayer_bridge::on_input(
        game,
        InputEvent::Scroll {
            at: screen.mouse,
            delta: scroll[1] as f32,
            mods: Modifiers::from_bits(modifiers),
        },
        canceled,
    );
}
