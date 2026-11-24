mod windows;
pub use self::windows::run;

/// Directory of the running executable (settings app and Vulkan layer live
/// next to it).
#[allow(dead_code)]
pub fn exe_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default()
}
