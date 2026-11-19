# Baut das Plugin nach dist/solidcraft-laserjoints/ und packt dist/solidcraft-laserjoints.zip.
# Braucht: Rust mit dem Target wasm32-unknown-unknown, und wasm-tools
# (https://github.com/bytecodealliance/wasm-tools/releases).
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

rustup target add wasm32-unknown-unknown | Out-Host
cargo build --release --lib --target wasm32-unknown-unknown
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$wasmTools = $env:WASM_TOOLS
if (-not $wasmTools) {
    $cmd = Get-Command wasm-tools -ErrorAction SilentlyContinue
    if ($cmd) { $wasmTools = $cmd.Source }
}
if (-not $wasmTools) {
    $local = Join-Path $PSScriptRoot "tools\wasm-tools.exe"
    if (Test-Path $local) { $wasmTools = $local }
}
if (-not $wasmTools) {
    throw "wasm-tools nicht gefunden. Lege es auf PATH, setze WASM_TOOLS oder kopiere wasm-tools.exe nach tools/."
}

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot "target" }
$core = Join-Path $targetDir "wasm32-unknown-unknown\release\solidcraft_laserjoints.wasm"
if (-not (Test-Path $core)) { throw "Core-Modul fehlt: $core" }

$out = Join-Path $PSScriptRoot "dist\solidcraft-laserjoints"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force -Path $out | Out-Null
& $wasmTools component new $core -o (Join-Path $out "plugin.wasm")
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Copy-Item "plugin.json", "README.md", "LICENSE" -Destination $out

$zip = Join-Path $PSScriptRoot "dist\solidcraft-laserjoints.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory($out, $zip)
Write-Host "built $out"
Write-Host "built $zip"
