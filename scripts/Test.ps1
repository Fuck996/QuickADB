# Copyright (C) 2026 QuickADB contributors
# SPDX-License-Identifier: GPL-3.0-only
# 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

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
