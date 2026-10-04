# Copyright (C) 2026 QuickADB contributors
# SPDX-License-Identifier: GPL-3.0-only
# 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

param([switch]$Release)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$registry = Join-Path $projectRoot 'vendor\registry'
if (!(Test-Path -LiteralPath $registry -PathType Container)) { throw '本脚本用于发布的完整源码包；仓库日常构建使用 Build.ps1。' }
$env:CARGO_HOME = Join-Path $projectRoot '.tools\source-cargo'
$env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
Push-Location $projectRoot
try {
    $arguments = @('build', '--target', 'x86_64-pc-windows-msvc', '--locked', '--offline')
    $arguments += @('--config', 'source.crates-io.replace-with="vendored-sources"')
    $arguments += @('--config', 'source.vendored-sources.directory="vendor/registry"')
    if ($Release) { $arguments += '--release' }
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw '完整源码包离线构建失败。' }
}
finally { Pop-Location }
