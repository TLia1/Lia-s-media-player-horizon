// Catalog hook adapter; the crate owns all UI behavior.
fn liasmediaplayer_screen_mouse_dragged_pre(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    button: i32,
    modifiers: u32,
    canceled: &mut bool,
) {
    use lias_media_player::platform::InputEvent;
    use lias_media_player::platform::Modifiers;
    crate::game::liasmediaplayer_bridge::on_input(
        game,
        InputEvent::Drag {
            at: screen.mouse,
            button,
            mods: Modifiers::from_bits(modifiers),
        },
        canceled,
    );
}
