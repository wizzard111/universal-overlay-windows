include!("../../resources/winres.rs");

fn main() {
    embed_resources(
        "Universal Overlay - installer",
        "install.exe",
        Level::RequireAdministrator,
    );
}
