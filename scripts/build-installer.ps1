# Builds the per-user Windows installer (NSIS) for the desktop app.
# Usage: powershell -File scripts\build-installer.ps1
# Output: target\release\bundle\nsis\ContextFlow_<version>_x64-setup.exe
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function Step($name, [scriptblock]$cmd) {
    Write-Host "==> $name" -ForegroundColor Cyan
    & $cmd
    if ($LASTEXITCODE -ne 0) { Write-Host "FAILED: $name" -ForegroundColor Red; exit $LASTEXITCODE }
}

$triple = ((rustc -vV | Select-String '^host: ').Line -replace '^host: ', '').Trim()

Step 'Build the command-line programs' { cargo build --release -p cf-cli -p cf-agent }

# Tauri ships "sidecar" programs named <name>-<target triple>.exe and installs
# them next to the app without the suffix.
$bin = 'app\src-tauri\binaries'
New-Item -ItemType Directory -Force $bin | Out-Null
foreach ($name in 'contextflow', 'contextflow-launch', 'contextflow-agent', 'contextflow-browser-host') {
    Copy-Item "target\release\$name.exe" "$bin\$name-$triple.exe" -Force
}

# Code signing (optional): set CONTEXTFLOW_SIGN_THUMBPRINT to the SHA-1 thumbprint of a
# code-signing certificate in the Windows certificate store. Without it the
# installer is built unsigned (SmartScreen will warn on first run).
$configs = @('--config', 'src-tauri/tauri.bundle.conf.json')
$signConfig = $null
if ($env:CONTEXTFLOW_SIGN_THUMBPRINT) {
    $timestamp = if ($env:CONTEXTFLOW_SIGN_TIMESTAMP_URL) { $env:CONTEXTFLOW_SIGN_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
    $signConfig = Join-Path $env:TEMP 'contextflow-sign.conf.json'
    @{ bundle = @{ windows = @{
        certificateThumbprint = $env:CONTEXTFLOW_SIGN_THUMBPRINT
        digestAlgorithm = 'sha256'
        timestampUrl = $timestamp
    } } } | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 $signConfig
    $configs += @('--config', $signConfig)
    Write-Host "Signing with certificate $($env:CONTEXTFLOW_SIGN_THUMBPRINT)" -ForegroundColor Cyan
} else {
    Write-Host 'No signing certificate set: the installer will be unsigned.' -ForegroundColor Yellow
}

Push-Location app
try {
    Step 'Install frontend packages' { npm ci }
    Step 'Build the installer' { npx tauri build @configs }
} finally {
    Pop-Location
    if ($signConfig) { Remove-Item $signConfig -ErrorAction SilentlyContinue }
}

Write-Host 'Installer:' -ForegroundColor Green
Get-ChildItem target\release\bundle\nsis\*.exe | ForEach-Object { Write-Host "  $($_.FullName)" }
