#Requires -Version 5.1
<#
  Universal Overlay — release build for Windows.

  Result (everything under source\target, nothing in the repository root):
    target\package\universal-overlay-windows-x86_64\
        install.exe  update.exe  uninstall.exe      <- the only files at the top
        app\         program, sensor service, PawnIO driver setup and modules
    target\dist\universal-overlay-windows-x86_64.zip   (what the updater downloads)
    target\dist\SHA256SUMS-windows.txt

  -Toolchain   rustup toolchain, e.g. stable-x86_64-pc-windows-gnu
  Environment:
    UO_GITHUB_REPO=owner/name            where the updater looks for releases
    WINDOWS_SIGN_PFX, WINDOWS_SIGN_PASSWORD
                                         optional Authenticode signing (osslsigncode
                                         if present, otherwise signtool)
#>
param([string]$Toolchain = '')

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$Source = Split-Path $PSScriptRoot
Set-Location $Source
$Name = 'universal-overlay-windows-x86_64'
$Pkg = Join-Path $Source "target\package\$Name"
$App = Join-Path $Pkg 'app'
$Dist = Join-Path $Source 'target\dist'
$Cache = Join-Path $Source 'target\pawnio-cache'

$version = (Select-String -Path 'Cargo.toml' -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "== Universal Overlay $version (Windows)"
if (-not $env:UO_GITHUB_REPO) { Write-Host '   UO_GITHUB_REPO not set: the updater uses wizzard111/universal-overlay-windows' }

# ---------------------------------------------------------------- build
$cargo = @('build', '--release', '--workspace')
if ($Toolchain) { $cargo = @("+$Toolchain") + $cargo }
& cargo @cargo
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
$bin = Join-Path $Source 'target\release'

# ---------------------------------------------------------------- PawnIO
New-Item -ItemType Directory -Force $Cache | Out-Null
$setup = Join-Path $Cache 'PawnIO_setup.exe'
if (-not (Test-Path $setup)) {
    Write-Host '== Downloading PawnIO_setup.exe'
    & curl.exe -fsSL -o $setup 'https://github.com/namazso/PawnIO.Setup/releases/latest/download/PawnIO_setup.exe'
    if ($LASTEXITCODE -ne 0) { throw 'PawnIO_setup.exe download failed' }
}
$mods = Join-Path $Cache 'modules'
if (-not (Test-Path (Join-Path $mods 'AMDFamily17.bin'))) {
    Write-Host '== Downloading PawnIO modules'
    $api = & curl.exe -fsSL 'https://api.github.com/repos/namazso/PawnIO.Modules/releases/latest' | ConvertFrom-Json
    $asset = $api.assets | Where-Object { $_.name -like '*.zip' } | Select-Object -First 1
    if (-not $asset) { throw 'no zip in the PawnIO.Modules release' }
    $zip = Join-Path $Cache 'modules.zip'
    & curl.exe -fsSL -o $zip $asset.browser_download_url
    $tmp = Join-Path $Cache 'modules-x'
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    Expand-Archive $zip $tmp
    New-Item -ItemType Directory -Force $mods | Out-Null
    foreach ($m in 'AMDFamily17.bin', 'IntelMSR.bin') {
        $f = Get-ChildItem $tmp -Recurse -Filter $m | Select-Object -First 1
        if (-not $f) { throw "no $m in the PawnIO modules" }
        Copy-Item $f.FullName $mods
    }
    $lic = Get-ChildItem $tmp -Recurse -File | Where-Object { $_.Name -match '^(COPYING|LICENSE)' } | Select-Object -First 1
    if ($lic) { Copy-Item $lic.FullName (Join-Path $mods 'COPYING') }
    Remove-Item -Recurse -Force $tmp, $zip
}

# ---------------------------------------------------------------- signing (optional)
function Sign-File([string]$path) {
    if (-not $env:WINDOWS_SIGN_PFX) { return }
    $ts = 'http://timestamp.digicert.com'
    if (Get-Command osslsigncode -ErrorAction SilentlyContinue) {
        $out = "$path.signed"
        & osslsigncode sign -pkcs12 $env:WINDOWS_SIGN_PFX -pass $env:WINDOWS_SIGN_PASSWORD -h sha256 -t $ts `
            -n 'Universal Overlay' -in $path -out $out
        if ($LASTEXITCODE -ne 0) { throw "signing failed: $path" }
        Move-Item -Force $out $path
    } else {
        & signtool sign /fd sha256 /tr $ts /td sha256 /f $env:WINDOWS_SIGN_PFX /p $env:WINDOWS_SIGN_PASSWORD /d 'Universal Overlay' $path
        if ($LASTEXITCODE -ne 0) { throw "signing failed: $path" }
    }
}

# ---------------------------------------------------------------- package
Remove-Item -Recurse -Force $Pkg -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $Dist, (Join-Path $App 'pawnio') | Out-Null
foreach ($f in 'universal-overlay.exe', 'universal-overlay-settings.exe', 'universal-overlay-updater.exe', 'uo-sensord.exe', 'uo-probe.exe') {
    Copy-Item (Join-Path $bin $f) $App
    Sign-File (Join-Path $App $f)
}
Copy-Item (Join-Path $bin 'uo-setup.exe') (Join-Path $Pkg 'install.exe')
Sign-File (Join-Path $Pkg 'install.exe')
Copy-Item (Join-Path $Pkg 'install.exe') (Join-Path $Pkg 'uninstall.exe')
Copy-Item (Join-Path $App 'universal-overlay-updater.exe') (Join-Path $Pkg 'update.exe')
Copy-Item $setup (Join-Path $App 'pawnio')
Copy-Item (Join-Path $mods '*') (Join-Path $App 'pawnio')
Copy-Item (Join-Path (Split-Path $Source) 'docs\README.md') $App
Set-Content -Path (Join-Path $App 'VERSION') -Value $version -Encoding ASCII -NoNewline

$zipPath = Join-Path $Dist "$Name.zip"
Remove-Item $zipPath -ErrorAction SilentlyContinue
Compress-Archive -Path $Pkg -DestinationPath $zipPath
$hash = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLower()
Set-Content -Path (Join-Path $Dist 'SHA256SUMS-windows.txt') -Value "$hash  $Name.zip" -Encoding ASCII
Write-Host "== Done: $zipPath"
Get-ChildItem $Pkg -Recurse -File | ForEach-Object { '{0,10:N0}  {1}' -f $_.Length, $_.FullName.Substring($Pkg.Length + 1) }
