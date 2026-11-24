@echo off
rem Universal Overlay: uninstall (same as Settings - Apps - Universal Overlay - Uninstall).
chcp 65001 >nul
setlocal
cd /d "%~dp0"

if exist "uninstall.exe" (
    start "" /wait "uninstall.exe" %*
    exit /b %errorlevel%
)
set "DIR=%ProgramFiles%\Universal Overlay"
for /f "tokens=2,*" %%a in ('reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\UniversalOverlay" /v InstallLocation 2^>nul ^| find "InstallLocation"') do set "DIR=%%b"
if not exist "%DIR%\uninstall.exe" (
    echo Universal Overlay не установлен.
    pause
    exit /b 1
)
start "" /wait "%DIR%\uninstall.exe" %*
