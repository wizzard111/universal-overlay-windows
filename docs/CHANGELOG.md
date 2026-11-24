# Changelog

## 0.1.1

* The whole program is in English: settings, overlay, tray menu, installer,
  updater and uninstaller.
* Updating from 0.1.0 replaces the old settings shortcut instead of leaving a
  second one.

## 0.1.0 — first release

* In-game overlay: FPS, frame time with a graph, 1% / 0.1% lows.
* Graphs of any sensor — as many as you like, each with its own color, height,
  history window and scale.
* Flexible styling: layout, font, colors, background, corner radius, overlay
  size 25–400%, opacity, screen corner and monitor.
* Settings app: dark, light or system theme, 8 accent colors, animations;
  changes apply to the overlay instantly.
* Automatic updates from GitHub with SHA-256 verification; the old version is
  removed completely, settings are kept.
* Lightweight: the hidden overlay uses no CPU; only displayed sensors are
  polled; between sensor ticks only the graph is redrawn and sent to the window.
* The overlay runs **without administrator rights**. The CPU temperature is
  read by the `uo-sensord` service (PawnIO driver), only when the visible
  overlay asks for it.
* Native `install.exe` / `uninstall.exe` instead of scripts: one UAC prompt,
  a setup log, Start menu, Installed apps, autostart.
* Every program has an icon, a manifest and version information.
* FPS from ETW (DXGI, D3D9, DxgKrnl — Vulkan and OpenGL), no injection.
