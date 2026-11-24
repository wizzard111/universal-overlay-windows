@echo off
rem Universal Overlay: update.
rem Source checkout with git: pull the latest code, rebuild, install in update mode.
rem Otherwise: run the updater, which downloads the latest GitHub release.
chcp 65001 >nul
setlocal
cd /d "%~dp0"

if exist ".git" (
    where git >nul 2>nul && where cargo >nul 2>nul && (
        echo Pulling the latest changes...
        git pull --ff-only || ( pause & exit /b 1 )
        powershell -NoProfile -ExecutionPolicy Bypass -File "source\build\build.ps1" || ( pause & exit /b 1 )
        start "" /wait "source\target\package\universal-overlay-windows-x86_64\install.exe" --update
        exit /b %errorlevel%
    )
)

if exist "update.exe" (
    "update.exe" %*
    exit /b %errorlevel%
)
set "DIR=%ProgramFiles%\Universal Overlay"
for /f "tokens=2,*" %%a in ('reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\UniversalOverlay" /v InstallLocation 2^>nul ^| find "InstallLocation"') do set "DIR=%%b"
if not exist "%DIR%\universal-overlay-updater.exe" (
    echo Universal Overlay is not installed. Run install.cmd.
    pause
    exit /b 1
)
"%DIR%\universal-overlay-updater.exe" %*
