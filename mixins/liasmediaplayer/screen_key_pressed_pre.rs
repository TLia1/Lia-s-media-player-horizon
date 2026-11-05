// Catalog hook adapter; the crate owns all UI behavior.
fn liasmediaplayer_screen_key_pressed_pre(
    game: &crate::game::Game<'_>,
    key: &str,
    modifiers: u32,
    canceled: &mut bool,
) {
    use lias_media_player::platform::InputEvent;
    use lias_media_player::platform::Modifiers;
    crate::game::liasmediaplayer_bridge::on_input(
        game,
        InputEvent::Key {
            key: lias_media_player::platform::Key::from_name(key),
            mods: Modifiers::from_bits(modifiers),
        },
        canceled,
    );
}
