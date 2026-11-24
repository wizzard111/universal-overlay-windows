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
        self.write(&format!("ВНИМАНИЕ: {msg}"));
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
    matches!(s.trim().to_lowercase().as_str(), "y" | "yes" | "д" | "да")
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
            "рядом с установщиком нет папки app с программой ({})",
            app.display()
        ));
    }
    let version = std::fs::read_to_string(app.join("VERSION"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let dir = &opts.install_dir;
    println!("\n  {APP_NAME} {version}\n  Папка: {}\n", dir.display());
    log.write(&format!(
        "{} {version} в {}",
        if opts.mode == Mode::Update {
            "Обновление"
        } else {
            "Установка"
        },
        dir.display()
    ));

    log.step("Останавливаю оверлей и службу датчиков");
    stop_overlay(PROCESSES);
    service_stop();

    if pawnio_installed() {
        log.step("Драйвер PawnIO уже установлен");
    } else {
        let setup = app.join("pawnio").join("PawnIO_setup.exe");
        if setup.is_file() {
            log.step("Устанавливаю драйвер PawnIO (температура CPU)");
            match run_wait(&setup, "-install -silent") {
                Ok(0) => {}
                Ok(code) => log.warn(&format!(
                    "установщик PawnIO вернул код {code} — температура CPU может быть недоступна"
                )),
                Err(e) => log.warn(&format!("PawnIO не установлен: {e}")),
            }
        } else {
            log.warn("в пакете нет PawnIO_setup.exe — температура CPU будет недоступна");
        }
    }

    log.step("Удаляю предыдущую версию");
    if remove_dir_robust(dir) > 0 {
        return Err(format!(
            "не удалось удалить {} — закройте программы из этой папки",
            dir.display()
        ));
    }
    log.step("Копирую файлы");
    let size = copy_dir(&app, dir).map_err(|e| format!("копирование: {e}"))?;
    let _ = std::fs::remove_file(dir.join("pawnio").join("PawnIO_setup.exe"));
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::copy(&me, dir.join("uninstall.exe")).map_err(|e| format!("uninstall.exe: {e}"))?;

    log.step("Регистрирую службу датчиков (температура CPU без прав администратора у оверлея)");
    if let Err(e) = service_install(&dir.join(uo_common::sensord::SERVICE_EXE)) {
        log.warn(&format!("служба: {e}"));
    }

    match add_to_performance_log_users() {
        Ok(true) => log.step("Пользователь добавлен в группу «Пользователи журналов производительности» — FPS заработает после повторного входа в Windows"),
        Ok(false) => log.step("Доступ к счётчикам кадров уже есть"),
        Err(e) => log.warn(&format!("группа «Пользователи журналов производительности»: {e}")),
    }

    log.step("Добавляю в меню «Пуск»");
    let menu = start_menu_dir();
    let overlay = dir.join(OVERLAY);
    com::shortcut(
        &menu.join(format!("{APP_NAME}.lnk")),
        &overlay,
        "",
        dir,
        "Оверлей FPS и датчиков",
    )?;
    com::shortcut(
        &menu.join(format!("{APP_NAME} — настройки.lnk")),
        &dir.join(SETTINGS),
        "",
        dir,
        "Настройки оверлея",
    )?;

    log.step("Регистрирую в «Установленных приложениях» и автозапуске");
    register_uninstall(dir, &version, size)?;
    set_autostart(Some(&overlay))?;

    log.step("Запускаю оверлей");
    launch_as_user(&overlay);
    log.write("Готово");
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
    println!("\n  Удаление {APP_NAME} из {}\n", dir.display());
    log.write(&format!("Удаление из {}", dir.display()));

    log.step("Останавливаю оверлей");
    stop_overlay(PROCESSES);
    log.step("Удаляю службу датчиков");
    service_remove();
    log.step("Удаляю автозапуск и ярлыки");
    let _ = set_autostart(None);
    let menu = start_menu_dir();
    let _ = std::fs::remove_file(menu.join(format!("{APP_NAME}.lnk")));
    let _ = std::fs::remove_file(menu.join(format!("{APP_NAME} — настройки.lnk")));
    log.step("Удаляю запись из «Установленных приложений»");
    unregister_uninstall();
    log.step("Удаляю файлы программы");
    if dir.join(OVERLAY).is_file() || dir.join("uninstall.exe").is_file() {
        let left = remove_dir_robust(dir);
        if left > 0 {
            log.warn(&format!(
                "{left} файл(ов) заняты — удалятся при перезагрузке"
            ));
        }
    }

    let cfg = uo_common::config::config_dir();
    if cfg.exists() {
        if opts.purge || ask(opts, "Удалить и настройки оверлея?") {
            let _ = std::fs::remove_dir_all(&cfg);
            log.step("Настройки удалены");
        } else {
            log.step(&format!("Настройки оставлены: {}", cfg.display()));
        }
    }
    if pawnio_installed()
        && ask(
            opts,
            "Удалить драйвер PawnIO? (его используют и FanControl, LibreHardwareMonitor)",
        )
    {
        log.step("Удаляю PawnIO");
        pawnio_uninstall();
    }
    if opts.stage2 {
        delete_at_reboot(&me);
    }
    log.write("Удалено");
    Ok(())
}

fn main() {
    let opts = parse_args();
    set_console_title(match opts.mode {
        Mode::Uninstall => "Universal Overlay — удаление",
        _ => "Universal Overlay — установка",
    });
    let mut log = Log::open();
    let result = match opts.mode {
        Mode::Uninstall => uninstall(&opts, &mut log),
        _ => install(&opts, &mut log),
    };
    match result {
        Ok(()) => {
            match opts.mode {
                Mode::Uninstall => println!("\n  {APP_NAME} удалён."),
                Mode::Update => println!("\n  Обновлено. Настройки сохранены."),
                Mode::Install => println!(
                    "\n  Готово. {APP_NAME} есть в меню «Пуск», значок — в трее, Ctrl+Alt+O — показать/скрыть.\n  Журнал: {}",
                    program_data().join(APP_NAME).join("install.log").display()
                ),
            }
            if opts.mode == Mode::Update {
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            pause(&opts, "Нажмите Enter, чтобы закрыть окно");
        }
        Err(e) => {
            println!("\n  ✗ {e}");
            log.write(&format!("ОШИБКА: {e}"));
            pause(
                &Opts {
                    quiet: false,
                    ..opts
                },
                "Нажмите Enter",
            );
            std::process::exit(1);
        }
    }
}
