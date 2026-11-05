// Runs `/show <image|video|audio> <url> [newPlayer]` on the client (`native_command`).
fn liasmediaplayer_command(
    game: &mut crate::game::Game<'_>,
    command: &str,
    ran: &mut bool,
) {
    crate::game::liasmediaplayer_bridge::on_command(game, command, ran);
}
