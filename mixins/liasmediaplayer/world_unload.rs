// Client level unload hook (`native_client_level_unload`).
fn liasmediaplayer_world_unload(game: &mut crate::game::Game<'_>, dimension: &str) {
    crate::game::liasmediaplayer_bridge::on_world_unload(game, dimension);
}
