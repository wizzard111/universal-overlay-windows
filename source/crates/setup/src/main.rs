//! Universal Overlay — installer and uninstaller for Windows (one program).
//!
//!   install.exe                  install / reinstall (next to the `app` folder)
//!   install.exe --update         quiet update (run by the updater)
//!   uninstall.exe                uninstall (also from Settings → Apps)
//!     --quiet                    no questions (settings and PawnIO are kept)
//!     --purge                    also remove the settings
//!   --install-dir DIR            another install folder
//!
//! The mode follows the file name ("uninstall" → uninstall), flags override
//! it. Administrator rights come from the manifest (one standard UAC
//! prompt). The console window is the installation log; the same lines go to
//! `%ProgramData%\Universal Overlay\install.log`.
//!
//! What the installer does — nothing more:
//!   * installs the signed PawnIO driver if missing (CPU temperature),
//!   * copies the program to Program Files, removing the previous version,
//!   * registers the small `uo-sensord` service that reads the CPU
//!     temperature, so the overlay itself never needs administrator rights,
//!   * adds the user to the built-in "Performance Log Users" group, which
//!     lets a normal program read the system's frame-present events (FPS),
//!   * Start menu shortcuts, an entry in Settings → Apps, autostart (Run key),
//!   * starts the overlay as the signed-in user (not elevated).
//!
//! User settings (%APPDATA%\UniversalOverlay) are never touched by install
//! or update.

mod com;
mod system;

use std::io::Write;
use std::path::{Path, PathBuf};
use system::*;

const APP_NAME: &str = "Universal Overlay";
const OVERLAY: &str = "universal-overlay.exe";
const SETTINGS: &str = "universal-overlay-settings.exe";
/// Start-menu shortcut name used by 0.1.0 (Russian UI); removed on install
/// and uninstall so an update does not leave two settings shortcuts.
const LEGACY_SETTINGS_LNK: &str = "Universal Overlay \u{2014} \u{43d}\u{430}\u{441}\u{442}\u{440}\u{43e}\u{439}\u{43a}\u{438}.lnk";
const PROCESSES: &[&str] = &[OVERLAY, SETTINGS, "uo-probe.exe"];

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Install,
    Update,
    Uninstall,
}

struct Opts {
    mode: Mode,
    quiet: bool,
    purge: bool,
    install_dir: PathBuf,
    /// Uninstall, second stage: running from a temporary copy.
    stage2: bool,
    /// Uninstall, second stage: the first stage's process, to wait for.
    parent_pid: Option<u32>,
}

fn parse_args() -> Opts {
    let args: Vec<String> = std::env::args().collect();
    let has = |a: &str| args.iter().any(|x| x == a);
    let exe_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    let mode = if has("--uninstall") || (exe_name.contains("uninstall") && !has("--install")) {
        Mode::Uninstall
    } else if has("--update") {
        Mode::Update
    } else {
        Mode::Install
    };
    let install_dir = args
        .iter()
        .position(|a| a == "--install-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .or_else(|| {
            (mode == Mode::Uninstall)
                .then(registered_install_dir)
                .flatten()
        })
        .unwrap_or_else(default_install_dir);
    Opts {
        mode,
        quiet: has("--quiet") || mode == Mode::Update,
        purge: has("--purge"),
        install_dir,
        stage2: has("--stage2"),
        parent_pid: args
            .iter()
            .position(|a| a == "--parent-pid")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok()),
    }
}

// ---------------------------------------------------------------- log

struct Log {
    file: Option<std::fs::File>,
}

impl Log {
    fn open() -> Self {
        let dir = program_data().join(APP_NAME);
        let _ = std::fs::create_dir_all(&dir);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("install.log"))
            .ok();
        Log { file }
    }

    fn step(&mut self, msg: &str) {
        println!("  • {msg}");
        self.write(msg);
    }

    fn warn(&mut self, msg: &str) {
        println!("  ! {msg}");
        self.write(&format!("WARNING: {msg}"));
    }

    fn write(&mut self, msg: &str) {
        if let Some(f) = self.file.as_mut() {
            let _ = writeln!(f, "[{}] {msg}", local_time());
        }
    }
}

fn ask(opts: &Opts, question: &str) -> bool {
    if opts.quiet {
        return false;
    }
    print!("  {question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    matches!(s.trim().to_lowercase().as_str(), "y" | "yes")
}

fn pause(opts: &Opts, msg: &str) {
    if !opts.quiet {
        print!("\n  {msg}");
        let _ = std::io::stdout().flush();
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}

// ---------------------------------------------------------------- install

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(to)?;
    let mut total = 0;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            total += copy_dir(&e.path(), &target)?;
        } else {
            total += std::fs::copy(e.path(), &target)?;
        }
    }
    Ok(total)
}

/// Remove `dir` file by file, retrying files that are still locked (a
/// process that is just exiting). Whatever stays locked is scheduled for
/// deletion at the next reboot. Returns the number of such leftovers.
fn remove_dir_robust(dir: &Path) -> usize {
    fn collect(dir: &Path, files: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    collect(&p, files, dirs);
                } else {
                    files.push(p);
                }
            }
        }
        dirs.push(dir.to_path_buf());
    }
    if !dir.exists() {
        return 0;
    }
    let (mut files, mut dirs) = (Vec::new(), Vec::new());
    collect(dir, &mut files, &mut dirs);
    for _ in 0..20 {
        files.retain(|f| std::fs::remove_file(f).is_err() && f.exists());
        if files.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    for f in &files {
        delete_at_reboot(f);
    }
    // Deepest directories first (collect pushes children before parents).
    for d in &dirs {
        if std::fs::remove_dir(d).is_err() && d.exists() {
            delete_at_reboot(d);
        }
    }
    files.len()
}

fn install(opts: &Opts, log: &mut Log) -> Result<(), String> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let app = exe_dir.join("app");
    if !app.join(OVERLAY).is_file() {
        return Err(format!(
            "no app folder next to the installer ({})",
            app.display()
        ));
    }
    let version = std::fs::read_to_string(app.join("VERSION"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let dir = &opts.install_dir;
    println!("\n  {APP_NAME} {version}\n  Folder: {}\n", dir.display());
    log.write(&format!(
        "{} {version} to {}",
        if opts.mode == Mode::Update {
            "Refresh"
        } else {
            "Installing"
        },
        dir.display()
    ));

    log.step("Stopping the overlay and the sensor service");
    stop_overlay(PROCESSES);
    service_stop();

    if pawnio_installed() {
        log.step("PawnIO driver is already installed");
    } else {
        let setup = app.join("pawnio").join("PawnIO_setup.exe");
        if setup.is_file() {
            log.step("Installing the PawnIO driver (CPU temperature)");
            match run_wait(&setup, "-install -silent") {
                Ok(0) => {}
                Ok(code) => log.warn(&format!(
                    "PawnIO setup returned code {code} — CPU temperature may be unavailable"
                )),
                Err(e) => log.warn(&format!("PawnIO not installed: {e}")),
            }
        } else {
            log.warn("PawnIO_setup.exe is missing from the package — CPU temperature will be unavailable");
        }
    }

    log.step("Removing the previous version");
    if remove_dir_robust(dir) > 0 {
        return Err(format!(
            "could not remove {} — close programs running from that folder",
            dir.display()
        ));
    }
    log.step("Copying files");
    let size = copy_dir(&app, dir).map_err(|e| format!("copying: {e}"))?;
    let _ = std::fs::remove_file(dir.join("pawnio").join("PawnIO_setup.exe"));
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::copy(&me, dir.join("uninstall.exe")).map_err(|e| format!("uninstall.exe: {e}"))?;

    log.step(
        "Registering the sensor service (CPU temperature without admin rights for the overlay)",
    );
    if let Err(e) = service_install(&dir.join(uo_common::sensord::SERVICE_EXE)) {
        log.warn(&format!("service: {e}"));
    }

    match add_to_performance_log_users() {
        Ok(true) => log.step("User added to the \"Performance Log Users\" group — FPS will work after signing in to Windows again"),
        Ok(false) => log.step("Frame counters access is already granted"),
        Err(e) => log.warn(&format!("\"Performance Log Users\" group: {e}")),
    }

    log.step("Adding to the Start menu");
    let menu = start_menu_dir();
    let _ = std::fs::remove_file(menu.join(LEGACY_SETTINGS_LNK));
    let overlay = dir.join(OVERLAY);
    com::shortcut(
        &menu.join(format!("{APP_NAME}.lnk")),
        &overlay,
        "",
        dir,
        "FPS and sensor overlay",
    )?;
    com::shortcut(
        &menu.join(format!("{APP_NAME} Settings.lnk")),
        &dir.join(SETTINGS),
        "",
        dir,
        "Overlay settings",
    )?;

    log.step("Registering in Installed apps and autostart");
    register_uninstall(dir, &version, size)?;
    set_autostart(Some(&overlay))?;

    log.step("Starting the overlay");
    launch_as_user(&overlay);
    log.write("Done");
    Ok(())
}

// ---------------------------------------------------------------- uninstall

fn uninstall(opts: &Opts, log: &mut Log) -> Result<(), String> {
    let dir = &opts.install_dir;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    // A program cannot delete its own folder while running from it:
    // continue from a temporary copy (deleted at the next reboot).
    if !opts.stage2 && me.starts_with(dir) {
        let tmp = std::env::temp_dir().join("universal-overlay-uninstall.exe");
        std::fs::copy(&me, &tmp).map_err(|e| e.to_string())?;
        let mut args: Vec<String> = std::env::args()
            .skip(1)
            .filter(|a| a != "--install-dir")
            .collect();
        args.retain(|a| Path::new(a) != dir.as_path());
        args.extend([
            "--uninstall".into(),
            "--stage2".into(),
            "--install-dir".into(),
            dir.display().to_string(),
        ]);
        args.extend(["--parent-pid".into(), std::process::id().to_string()]);
        // Do not wait: this process must exit so its own .exe can be deleted.
        std::process::Command::new(&tmp)
            .args(&args)
            .spawn()
            .map_err(|e| e.to_string())?;
        std::process::exit(0);
    }
    if let Some(pid) = opts.parent_pid {
        wait_for_exit(pid, 5000);
    }
    println!("\n  Uninstalling {APP_NAME} from {}\n", dir.display());
    log.write(&format!("Uninstalling from {}", dir.display()));

    log.step("Stopping the overlay");
    stop_overlay(PROCESSES);
    log.step("Removing the sensor service");
    service_remove();
    log.step("Removing autostart and shortcuts");
    let _ = set_autostart(None);
    let menu = start_menu_dir();
    let _ = std::fs::remove_file(menu.join(format!("{APP_NAME}.lnk")));
    let _ = std::fs::remove_file(menu.join(format!("{APP_NAME} Settings.lnk")));
    let _ = std::fs::remove_file(menu.join(LEGACY_SETTINGS_LNK));
    log.step("Removing the Installed apps entry");
    unregister_uninstall();
    log.step("Removing program files");
    if dir.join(OVERLAY).is_file() || dir.join("uninstall.exe").is_file() {
        let left = remove_dir_robust(dir);
        if left > 0 {
            log.warn(&format!(
                "{left} file(s) in use — they will be removed at the next restart"
            ));
        }
    }

    let cfg = uo_common::config::config_dir();
    if cfg.exists() {
        if opts.purge || ask(opts, "Remove the overlay settings too?") {
            let _ = std::fs::remove_dir_all(&cfg);
            log.step("Settings removed");
        } else {
            log.step(&format!("Settings kept: {}", cfg.display()));
        }
    }
    if pawnio_installed()
        && ask(
            opts,
            "Remove the PawnIO driver? (FanControl, LibreHardwareMonitor and others use it too)",
        )
    {
        log.step("Removing PawnIO");
        pawnio_uninstall();
    }
    if opts.stage2 {
        delete_at_reboot(&me);
    }
    log.write("Uninstalled");
    Ok(())
}

fn main() {
    let opts = parse_args();
    set_console_title(match opts.mode {
        Mode::Uninstall => "Universal Overlay — uninstall",
        _ => "Universal Overlay — setup",
    });
    let mut log = Log::open();
    let result = match opts.mode {
        Mode::Uninstall => uninstall(&opts, &mut log),
        _ => install(&opts, &mut log),
    };
    match result {
        Ok(()) => {
            match opts.mode {
                Mode::Uninstall => println!("\n  {APP_NAME} has been uninstalled."),
                Mode::Update => println!("\n  Updated. Your settings were kept."),
                Mode::Install => println!(
                    "\n  Done. {APP_NAME} is in the Start menu, its icon is in the tray, Ctrl+Alt+O shows/hides it.\n  Log: {}",
                    program_data().join(APP_NAME).join("install.log").display()
                ),
            }
            if opts.mode == Mode::Update {
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            pause(&opts, "Press Enter to close this window");
        }
        Err(e) => {
            println!("\n  ✗ {e}");
            log.write(&format!("ERROR: {e}"));
            pause(
                &Opts {
                    quiet: false,
                    ..opts
                },
                "Press Enter",
            );
            std::process::exit(1);
        }
    }
}
