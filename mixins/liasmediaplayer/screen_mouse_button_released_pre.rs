// Catalog hook adapter; the crate owns all UI behavior.
fn liasmediaplayer_screen_mouse_button_released_pre(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    use lias_media_player::platform::InputEvent;
    crate::game::liasmediaplayer_bridge::on_input(
        game,
        InputEvent::Up {
            at: screen.mouse,
            button,
        },
        canceled,
    );
}
