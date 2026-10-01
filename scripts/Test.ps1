$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$env:CARGO_HOME = Join-Path $projectRoot '.tools\cargo'
$env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
Push-Location $projectRoot
try {
    & cargo test --target x86_64-pc-windows-msvc --locked
    if ($LASTEXITCODE -ne 0) { throw '测试失败。' }
}
finally { Pop-Location }
