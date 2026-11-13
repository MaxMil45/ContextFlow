# Runs the same checks as CI. Usage: powershell -File scripts\check.ps1
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function Step($name, [scriptblock]$cmd) {
    Write-Host "==> $name" -ForegroundColor Cyan
    & $cmd
    if ($LASTEXITCODE -ne 0) { Write-Host "FAILED: $name" -ForegroundColor Red; exit $LASTEXITCODE }
}

Step 'Frontend' { Push-Location app; npm ci; if ($LASTEXITCODE -eq 0) { npm run build }; Pop-Location }
Step 'Format'  { cargo fmt --all -- --check }
Step 'Clippy'  { cargo clippy --workspace --all-targets -- -D warnings }
Step 'Test'    { cargo test --workspace }
Write-Host 'All checks passed.' -ForegroundColor Green
