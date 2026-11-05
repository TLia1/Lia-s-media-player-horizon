// Enrich only our client-owned command after the host has merged native commands.
fn liasmediaplayer_command_tree(
    mut tree: horizon_client::command::Dispatcher,
    base: &horizon_client::command::Dispatcher,
    names: &[&str],
) -> horizon_client::command::Dispatcher {
    use horizon_client::command::{ArgType, MOD_COMMAND, Suggests};
    if !names.contains(&"show") || base.find("show").is_some() {
        return tree;
    }
    let Some(show) = tree.find("show") else { return tree; };
    // Keep the greedy fallback for malformed arguments and private chat click links.
    for kind in lias_media_player::command::ShowKind::ALL {
        let kind = tree.literal(show, kind.id());
        tree.executes(kind, MOD_COMMAND);
        // ScoreHolder consumes an entire whitespace-delimited token, unlike Word
        // which stops at URL punctuation. Nothing suppresses player-name suggestions.
        let url = tree.argument(kind, "url", ArgType::ScoreHolder { multiple: false });
        tree.suggests(url, Suggests::Nothing);
        tree.executes(url, MOD_COMMAND);
        let new_player = tree.argument(url, "newPlayer", ArgType::Bool);
        tree.executes(new_player, MOD_COMMAND);
    }
    tree
}
