# Copyright (C) 2026 QuickADB contributors
# SPDX-License-Identifier: GPL-3.0-only
# 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

param(
    [string]$Revision = 'HEAD',
    [string]$BinaryRevision,
    [string]$RegistryDirectory
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$env:CARGO_HOME = Join-Path $projectRoot '.tools\cargo'
Push-Location $projectRoot
try {
    $commit = & git rev-parse --verify "$Revision^{commit}"
    if ($LASTEXITCODE -ne 0) { throw '源码版本不是有效的已提交版本。' }
    $manifest = & git show "${commit}:Cargo.toml"
    if ($LASTEXITCODE -ne 0 -or ($manifest -join "`n") -notmatch '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"') {
        throw '无法读取该提交的应用版本。'
    }
    $version = $Matches[1]
    $artifacts = Join-Path $projectRoot 'artifacts'
    $stage = Join-Path $artifacts ("source-$version-" + $commit.Substring(0, 8))
    if (Test-Path -LiteralPath $stage) { throw "源码暂存目录已存在：$stage" }
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    $tracked = Join-Path $stage 'tracked.zip'
    & git archive --format=zip "--output=$tracked" $commit
    if ($LASTEXITCODE -ne 0) { throw '导出已提交源码失败。' }
    $sourceRoot = Join-Path $stage "QuickADB-$version-source"
    Expand-Archive -LiteralPath $tracked -DestinationPath $sourceRoot
    Remove-Item -LiteralPath $tracked
    if ($BinaryRevision) {
        $binaryCommit = & git rev-parse --verify "$BinaryRevision^{commit}"
        if ($LASTEXITCODE -ne 0) { throw '无法确认 EXE 对应提交。' }
    } else { $binaryCommit = $commit }
    $executable = Join-Path $artifacts "QuickADB-$version-x64.exe"
    if (!(Test-Path -LiteralPath $executable -PathType Leaf)) { throw '缺少本版本已验证的发布 EXE。' }
    $registryTarget = Join-Path $sourceRoot 'vendor\registry'
    if ($RegistryDirectory) {
        Copy-Item -LiteralPath $RegistryDirectory -Destination $registryTarget -Recurse
    } else {
        Push-Location $sourceRoot
        try {
            & cargo vendor --locked --respect-source-config --versioned-dirs --no-delete --quiet 'vendor/registry'
            if ($LASTEXITCODE -ne 0) { throw '获取锁定依赖源码失败。' }
        }
        finally { Pop-Location }
    }
    [pscustomobject]@{
        Version = $version
        SourceCommit = $commit
        BinaryCommit = $binaryCommit
        Executable = Split-Path $executable -Leaf
        ExecutableSHA256 = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
        License = 'GPL-3.0-only; attribution term in NOTICE under section 7(b)'
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $sourceRoot 'SOURCE_INFO.json') -Encoding utf8NoBOM
    $sourcePath = $sourceRoot.Replace('\', '/')
    $packageCargoHome = $env:CARGO_HOME
    $env:CARGO_HOME = Join-Path $projectRoot ".tools\source-check-$($commit.Substring(0, 8))"
    try {
        $metadata = & cargo metadata --manifest-path "$sourceRoot\Cargo.toml" --format-version 1 --locked --offline --filter-platform x86_64-pc-windows-msvc --config "source.crates-io.replace-with='vendored-sources'" --config "source.vendored-sources.directory='$sourcePath/vendor/registry'"
        if ($LASTEXITCODE -ne 0) { throw '源码包的锁定依赖离线验证失败。' }
    }
    finally { $env:CARGO_HOME = $packageCargoHome }
    $resolved = $metadata | ConvertFrom-Json
    foreach ($package in $resolved.packages) {
        if (![IO.Path]::GetFullPath($package.manifest_path).StartsWith($sourceRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw "发现源码包以外的依赖：$($package.name)"
        }
    }
    $fileName = "QuickADB-$version-source.zip"
    $archive = Join-Path $artifacts $fileName
    if (Test-Path -LiteralPath $archive) { throw "源码包已存在，不自动覆盖：$archive" }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($stage, $archive, [IO.Compression.CompressionLevel]::Optimal, $false)
    [pscustomobject]@{
        File = $archive
        SourceCommit = $commit
        BinaryCommit = $binaryCommit
        Bytes = (Get-Item -LiteralPath $archive).Length
        SHA256 = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
        Dependencies = $resolved.packages.Count - 1
    } | ConvertTo-Json
}
finally { Pop-Location }
