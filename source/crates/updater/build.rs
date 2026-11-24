include!("../../resources/winres.rs");

fn main() {
    embed_resources(
        "Universal Overlay - updater",
        "universal-overlay-updater.exe",
        Level::AsInvoker,
    );
}
