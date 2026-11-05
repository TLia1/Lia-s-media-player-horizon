// Gameplay key hook (`native_key`).
fn liasmediaplayer_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    crate::game::liasmediaplayer_bridge::on_gameplay_key(game, key, pressed);
}
