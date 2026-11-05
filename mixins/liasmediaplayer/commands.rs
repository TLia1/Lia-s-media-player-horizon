// Registers client-owned commands (`/show`) in Horizon's command tree (`native_commands`).
fn liasmediaplayer_commands(
    game: &mut crate::game::Game<'_>,
    commands: &mut Vec<&'static str>,
) {
    crate::game::liasmediaplayer_bridge::on_commands(game, commands);
}
