pub extern crate horizon_command;
extern crate lias_media_player;
mod horizon_client { pub use horizon_command as command; }
include!("../mixins/liasmediaplayer/command_tree.rs");
fn main() {
    let base = horizon_command::Dispatcher::new();
    let tree = liasmediaplayer_command_tree(base.with_mod_commands(["show"]), &base, &["show"]);
    let reg = horizon_command::Registries::default();
    for (input, expected) in [
        ("show ", vec!["audio", "gif", "image", "video"]),
        ("show vi", vec!["video"]),
        ("show video https://example.test/video.mp4 ", vec!["false", "true"]),
        ("show audio https://youtu.be/id?list=example tr", vec!["true"]),
    ] {
        let parse = tree.parse(input, &reg);
        let (_, suggestions) = tree.client_suggestions(&parse, input.len(), &reg);
        let actual: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(actual, expected, "{input}");
    }
    println!("Native command suggestions: 4 checks passed");
}
