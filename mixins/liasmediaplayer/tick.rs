// Client tick hook (`native_tick`).
fn liasmediaplayer_tick(game: &mut crate::game::Game<'_>) {
    crate::game::liasmediaplayer_bridge::on_tick(game);
}
