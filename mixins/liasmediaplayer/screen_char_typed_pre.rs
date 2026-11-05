// Catalog hook adapter; the crate owns all UI behavior.
fn liasmediaplayer_screen_char_typed_pre(
    game: &crate::game::Game<'_>,
    c: char,
    canceled: &mut bool,
) {
    use lias_media_player::platform::InputEvent;
    crate::game::liasmediaplayer_bridge::on_input(game, InputEvent::Char(c), canceled);
}
