# Copyright (C) 2026 QuickADB contributors
# SPDX-License-Identifier: GPL-3.0-only
# 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$env:CARGO_HOME = Join-Path $projectRoot '.tools\cargo'
Push-Location $projectRoot
try {
    $metadataText = & cargo metadata --format-version 1 --locked --offline --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw '无法读取锁定依赖的许可信息。' }
    $metadata = $metadataText | ConvertFrom-Json
    $supplements = Get-Content -LiteralPath (Join-Path $projectRoot 'assets\licenses\registry-notices.json') -Raw | ConvertFrom-Json -AsHashtable
    $included = @{}
    foreach ($node in $metadata.resolve.nodes) { $included[$node.id] = $true }
    $text = [System.Text.StringBuilder]::new()
    [void]$text.AppendLine('QuickADB 开源组件许可')
    [void]$text.AppendLine('以下内容随应用内嵌，可在设置中复制。组件保留各自的许可证。')
    foreach ($package in ($metadata.packages | Sort-Object name, version)) {
        if (!$included.ContainsKey($package.id) -or $package.name -eq 'quickadb') { continue }
        $directory = Split-Path $package.manifest_path -Parent
        [void]$text.AppendLine("`n=== $($package.name) $($package.version) ===")
        [void]$text.AppendLine("License: $($package.license)")
        [void]$text.AppendLine("Source: $($package.repository)")
        $files = @(Get-ChildItem -LiteralPath $directory -File | Where-Object { $_.Name -match '^(LICENSE|COPYING|NOTICE|UNLICENSE)' })
        if ($package.name -like 'droidmux*') {
            $files += Get-Item -LiteralPath (Join-Path $projectRoot 'vendor\droidmux\LICENSE-MIT'), (Join-Path $projectRoot 'vendor\droidmux\LICENSE-APACHE')
        }
        if ($package.name -eq 'libusb1-sys') {
            $files += Get-Item -LiteralPath (Join-Path $directory 'libusb\COPYING')
            [void]$text.AppendLine('Bundled libusb: LGPL-2.1-or-later; https://github.com/libusb/libusb')
        }
        if ($package.name -eq 'epaint_default_fonts') {
            $files += Get-ChildItem -LiteralPath (Join-Path $directory 'fonts') -File | Where-Object { $_.Name -match '(LICENSE|COPYING)' }
        }
        if ($files.Count -eq 0) {
            $key = "$($package.name)@$($package.version)"
            if (!$supplements.ContainsKey($key)) { throw "组件缺少许可全文，请核对官方来源：$key" }
            foreach ($entry in $supplements[$key].files) {
                $file = Get-Item -LiteralPath (Join-Path $projectRoot "assets\licenses\$($entry.path)")
                if ((Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ne $entry.sha256) {
                    throw "补充许可原文校验失败：$key / $($entry.path)"
                }
                [void]$text.AppendLine("License source: $($entry.source)")
                $files += $file
            }
        }
        foreach ($file in ($files | Sort-Object FullName -Unique)) {
            [void]$text.AppendLine("--- $($file.Name) ---")
            [void]$text.AppendLine([System.IO.File]::ReadAllText($file.FullName))
        }
    }
    $output = Join-Path $projectRoot 'assets\ThirdPartyNotices.txt'
    $content = $text.ToString() -replace '(?m)[ \t]+\r?$', ''
    [System.IO.File]::WriteAllText($output, $content, [System.Text.UTF8Encoding]::new($false))
    Write-Output "已生成内嵌许可：$output"
}
finally { Pop-Location }
