param([switch]$Release)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$env:CARGO_HOME = Join-Path $projectRoot '.tools\cargo'
$env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
New-Item -ItemType Directory -Force -Path $env:CARGO_HOME | Out-Null
$registryConfig = Join-Path $env:CARGO_HOME 'config.toml'
if (!(Test-Path -LiteralPath $registryConfig)) {
    @'
[source.crates-io]
replace-with = "rsproxy-sparse"
[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"
'@ | Set-Content -LiteralPath $registryConfig -Encoding utf8
}
Push-Location $projectRoot
try {
    $arguments = @('build', '--target', 'x86_64-pc-windows-msvc')
    if ($Release) { $arguments += '--release' }
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw '原生构建失败。' }
}
finally { Pop-Location }
