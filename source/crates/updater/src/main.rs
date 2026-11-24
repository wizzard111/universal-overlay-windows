//! Universal Overlay updater.
//!
//!   universal-overlay-updater --check     print "latest X", "available X" or "error …"
//!                                          (exit code 0 = up to date, 10 = update available)
//!   universal-overlay-updater             download and install the latest release
//!     --install-dir PATH                   update this installation (default: the
//!                                          installed copy, see `platform::install_dir`)
//!
//! Network and unpacking use the OS's own `curl` and `tar` (present on
//! Windows 10+, macOS and Linux), so no HTTP/TLS stack is compiled in.
//! The download is verified against the release's SHA256SUMS.txt.
//! Installing = running the *new* package's installer in update mode: it
//! removes the old release completely and keeps the user's settings (they
//! live in the user config directory, not in the install directory).
//!
//! Works both from the installed program and from an unpacked release
//! folder (its "update" entry): it always updates the installed copy and
//! compares the *installed* version.

#[path = "platform_windows.rs"]
mod platform;

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use uo_common::{release_asset_name, version_newer, GITHUB_REPO, VERSION};

struct Release {
    version: String,
    asset_url: String,
    sums_url: Option<String>,
}

/// Print and append to `<config dir>/update.log` (the updater is often
/// started from the GUI, where stdout goes nowhere).
fn log(msg: &str) {
    println!("{msg}");
    let path = uo_common::config::config_dir().join("update.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = writeln!(f, "[{}] {msg}", VERSION);
    }
}

fn arg_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn has_arg(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn curl() -> Command {
    let mut c = Command::new("curl.exe");
    c.args([
        "-fsSL",
        "--retry",
        "2",
        "--connect-timeout",
        "15",
        "-A",
        "universal-overlay-updater",
    ]);
    platform::no_window(&mut c);
    c
}

fn fetch_latest(repo: &str) -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let out = curl()
        .args(["-H", "Accept: application/vnd.github+json", &url])
        .output()
        .map_err(|e| format!("curl not found: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "GitHub is unreachable ({})",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let version = v["tag_name"]
        .as_str()
        .ok_or("the release has no tag_name")?
        .trim_start_matches(['v', 'V'])
        .to_string();
    let want = release_asset_name();
    let mut asset_url = None;
    let mut sums_url = None;
    for a in v["assets"].as_array().into_iter().flatten() {
        let name = a["name"].as_str().unwrap_or_default();
        let url = a["browser_download_url"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if name == want {
            asset_url = Some(url);
        } else if name == "SHA256SUMS.txt" {
            sums_url = Some(url);
        }
    }
    let asset_url = asset_url.ok_or_else(|| format!("release {version} has no file {want}"))?;
    Ok(Release {
        version,
        asset_url,
        sums_url,
    })
}

fn download(url: &str, to: &Path) -> Result<(), String> {
    let st = curl()
        .args(["-o"])
        .arg(to)
        .arg(url)
        .status()
        .map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("could not download {url}"))
    }
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// The installation to update.
fn install_dir() -> PathBuf {
    arg_value("--install-dir")
        .map(PathBuf::from)
        .unwrap_or_else(platform::install_dir)
}

/// Version of the installed copy (its VERSION file), else our own.
fn installed_version(dir: &Path) -> String {
    platform::version_file(dir)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| VERSION.to_string())
}

fn work_dir(version: &str) -> PathBuf {
    std::env::temp_dir().join(format!("universal-overlay-update-{version}"))
}

/// Remove leftovers of earlier updates.
fn clean_old_work_dirs(keep: &Path) {
    if let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p != keep
                && e.file_name()
                    .to_string_lossy()
                    .starts_with("universal-overlay-update-")
            {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
    }
}

fn update(repo: &str) -> Result<String, String> {
    let install = install_dir();
    let current = installed_version(&install);
    let rel = fetch_latest(repo)?;
    if !version_newer(&rel.version, &current) {
        return Ok(format!("The latest version {current} is installed"));
    }
    let work = work_dir(&rel.version);
    if !has_arg("--staged") {
        clean_old_work_dirs(&work);
        let _ = std::fs::remove_dir_all(&work);
    }
    std::fs::create_dir_all(work.join("x")).map_err(|e| e.to_string())?;

    // The installer replaces this very executable; where the OS cannot
    // overwrite a running file, continue from a temporary copy.
    if !has_arg("--staged") {
        if let Some(result) = platform::restart_staged(&work, repo, &install) {
            return result;
        }
    }

    log(&format!(
        "Downloading version {} (installed: {current})…",
        rel.version
    ));
    let archive = work.join(release_asset_name());
    download(&rel.asset_url, &archive)?;
    match &rel.sums_url {
        Some(u) => {
            let sums = work.join("SHA256SUMS.txt");
            download(u, &sums)?;
            let text = std::fs::read_to_string(&sums).map_err(|e| e.to_string())?;
            let name = release_asset_name();
            let want = text
                .lines()
                .find_map(|l| {
                    let mut it = l.split_whitespace();
                    let (h, f) = (it.next()?, it.next()?);
                    (f.trim_start_matches('*') == name).then(|| h.to_lowercase())
                })
                .ok_or("SHA256SUMS.txt does not list the file")?;
            if sha256_hex(&archive)? != want {
                return Err("checksum mismatch — the file is damaged or was tampered with".into());
            }
        }
        None => {
            return Err(
                "the release has no SHA256SUMS.txt — unverified updates are not allowed".into(),
            )
        }
    }

    let x = work.join("x");
    let mut tar = Command::new("tar.exe");
    platform::no_window(&mut tar);
    let st = tar
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&x)
        .status()
        .map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("could not unpack the archive".into());
    }
    // The archive contains a single top-level directory.
    let pkg = std::fs::read_dir(&x)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or("empty archive")?;

    log("Installing…");
    platform::run_installer(&pkg, &install)?;
    let _ = std::fs::remove_file(&archive);
    let _ = std::fs::remove_dir_all(&x);
    Ok(format!("Updated to version {}", rel.version))
}

fn main() -> ExitCode {
    let repo = arg_value("--repo").unwrap_or_else(|| GITHUB_REPO.to_string());
    if has_arg("--version") {
        println!("{VERSION}");
        return ExitCode::SUCCESS;
    }
    if has_arg("--check") {
        let current = installed_version(&install_dir());
        return match fetch_latest(&repo) {
            Ok(r) if version_newer(&r.version, &current) => {
                println!("available {}", r.version);
                ExitCode::from(10)
            }
            Ok(_) => {
                println!("latest {current}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                println!("error {e}");
                ExitCode::FAILURE
            }
        };
    }
    let code = match update(&repo) {
        Ok(msg) => {
            if !msg.is_empty() {
                log(&msg);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            log(&format!("Update failed: {e}"));
            ExitCode::FAILURE
        }
    };
    platform::pause_if_console();
    code
}
