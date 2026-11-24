@echo off
rem Universal Overlay: install from this folder.
rem In a release archive: runs install.exe next to the "app" folder.
rem In a source checkout: builds the program first (needs Rust), then installs.
chcp 65001 >nul
setlocal
cd /d "%~dp0"

if exist "app\universal-overlay.exe" if exist "install.exe" (
    start "" /wait "install.exe" %*
    exit /b %errorlevel%
)

set "PKG=source\target\package\universal-overlay-windows-x86_64"
if not exist "%PKG%\install.exe" (
    echo Сборка Universal Overlay из исходников...
    where cargo >nul 2>nul || (
        echo Нужен Rust: https://rustup.rs  ^(или скачайте готовый релиз^)
        pause
        exit /b 1
    )
    powershell -NoProfile -ExecutionPolicy Bypass -File "source\build\build.ps1" || (
        echo Сборка не удалась.
        pause
        exit /b 1
    )
)
start "" /wait "%PKG%\install.exe" %*
exit /b %errorlevel%
