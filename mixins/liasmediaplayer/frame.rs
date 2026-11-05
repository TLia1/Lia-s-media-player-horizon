// Per-frame hook (`native_frame`).
fn liasmediaplayer_frame(game: &mut crate::game::Game<'_>, elapsed: std::time::Duration) {
    crate::game::liasmediaplayer_bridge::on_frame(game, elapsed);
}
