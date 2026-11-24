# Universal Overlay for Windows

A lightweight in-game overlay: FPS, frame time with a graph, 1% / 0.1% lows,
graphs of any sensor (load, clocks, GPU/CPU temperatures, memory), flexible
styling and size, and automatic updates from GitHub.

**[Download the latest version](https://github.com/wizzard111/universal-overlay-windows/releases/latest)** —
`universal-overlay-windows-x86_64.zip`: unpack it and run `install.exe`.

Other systems: [Linux](https://github.com/wizzard111/universal-overlay-linux) · [macOS](https://github.com/wizzard111/universal-overlay-macos).

License: [MIT](../LICENSE). What's new: [CHANGELOG](CHANGELOG.md). Built together with Claude.

## Install, update, uninstall

The release archive has only three programs at the top; everything else is in `app\`:

| File | What it does |
|---|---|
| `install.exe` | Installs or reinstalls. One UAC prompt; the window is the setup log |
| `update.exe` | Downloads the latest GitHub release, verifies its SHA-256 and updates the installed program. The old version is removed completely; your settings are kept |
| `uninstall.exe` | Removes the program, service, shortcuts and autostart; asks about your settings and the driver |

In the repository these are `install.cmd`, `update.cmd` and `uninstall.cmd`:
from source they build the program (needs [Rust](https://rustup.rs)) and run
the same steps. You can also uninstall from Settings → Apps and update from
the app (General → Updates).

What the installer does — and nothing else:
* installs the signed [PawnIO](https://pawnio.eu) driver for the CPU
  temperature (skipped if it is already there, e.g. from FanControl or
  LibreHardwareMonitor);
* puts the program into `C:\Program Files\Universal Overlay`;
* registers the small `uo-sensord` service (see below);
* adds the user to the built-in Windows group "Performance Log Users", which
  lets a normal program read the system's frame-present events (FPS). It
  takes effect after signing in to Windows again;
* adds Start menu shortcuts, an entry in Installed apps and autostart.

The overlay itself **runs without administrator rights**.

## Antivirus friendliness

The overlay is built like a program with nothing to hide:
* it never elevates and never asks to "run as administrator";
* it is installed by a regular installer with a manifest, not by a script;
* every executable has an icon, a manifest and version information;
* nothing is injected into games: FPS comes from ETW system events (the same
  source as Intel PresentMon), the temperature from a signed driver.

Heuristic false positives can never be ruled out completely for unsigned
software. The real fix is code signing: the build signs every executable when
`WINDOWS_SIGN_PFX` and `WINDOWS_SIGN_PASSWORD` are set (in CI — secrets with
the same names). Open-source projects can get a free certificate, e.g. from
the [SignPath Foundation](https://signpath.org). If a scanner still flags it,
please report a false positive to its vendor.

## CPU temperature: the uo-sensord service

The PawnIO driver can only be opened with full rights, so the sensor is read
not by the overlay but by the `uo-sensord` service (~300 KB). It waits in the
kernel and uses no CPU. While the overlay is visible, it signals "reading
needed" once per tick; the service reads the register with a single
`DeviceIoControl` call (AMD Zen 1–5: `THM_TCON_CUR_TMP`, Intel:
`IA32_PACKAGE_THERM_STATUS`), puts the value into shared memory and replies.
That takes microseconds and the value is always fresh. No WMI, no .NET, no
LibreHardwareMonitor.

## Features

* **Frame time and FPS** from ETW: DXGI (D3D10–12), D3D9, DxgKrnl (Vulkan,
  OpenGL and the rest). No injection. A graph that highlights bad frames;
  spikes are never lost: each pixel shows the slowest frame.
* **1% / 0.1% lows** — percentile or average of the worst frames, with
  configurable windows.
* **Graphs of any sensor** — as many as you like, each with its own color,
  height, history window and scale.
* **Sensors:** GPU — load, clocks, temperatures (core, hotspot, memory), power,
  voltage, fan, VRAM (AMD ADL, NVIDIA NVML, Windows counters for the rest);
  CPU — load, clock, temperature; RAM.
* **Styling:** column or row layout, font, sizes, per-sensor colors,
  background, corner radius, spacing, **overlay size** (25–400%),
  **opacity**, screen corner and offset, game monitor or primary monitor.
* **Settings app** — dark, light or system theme, 8 accent colors,
  animations; changes apply to the overlay instantly.

The default hotkey is **Ctrl+Alt+O**. `universal-overlay --toggle` toggles the
overlay (and starts it if it is not running).

## Resource use

| State | CPU | Memory |
|---|---|---|
| Visible, measuring the active game's FPS, graph at 30 Hz | ≈0.26% of one core | ~26 MB |
| Visible, no frames | ≈0.08% of one core | ~26 MB |
| Hidden (tray icon only) | 0: no timers, frame capture stopped | ~2 MB |
| uo-sensord service | 0 until a temperature is requested | ~8 MB |

Measured on a Ryzen 5 5600X, Radeon RX 9060 XT, Windows 11.

What keeps it light:
* only sensors that are shown or graphed are polled;
* between sensor ticks only the graph is redrawn, and only its rectangle is
  sent to the window (`UpdateLayeredWindowIndirect` with a dirty region);
* lows are computed by O(n) selection, only the needed tail of the frame
  history is copied;
* sensor graphs are redrawn once per tick; rendering is software-only with no
  GPU context; icons are compressed PNGs of a few KB.

## Limitations

The overlay is not visible over games in **exclusive** full-screen mode (old
DX9/DX11 titles). Use borderless windowed or modern flip-model full screen.

## Building

```
install.cmd                                   build and install
powershell -File source\build\build.ps1       build a release only → source\target\dist
```

Pushing a `v*` tag runs `.github/workflows/release.yml`: tests, build and a
GitHub release with `SHA256SUMS.txt`.

## Layout

```
install.cmd  update.cmd  uninstall.cmd    install, update, uninstall
docs/                                     documentation
source/
  crates/common     config, hotkeys, fonts, sensor-service protocol
  crates/overlay    overlay: core, renderer, frames (ETW), sensors, tray
  crates/settings   settings app
  crates/updater    updater (Windows' own curl and tar, SHA-256 check)
  crates/sensord    CPU temperature service (PawnIO)
  crates/setup      installer / uninstaller
  resources/        icon and shared executable resources (manifest, version)
  build/            release build
```
