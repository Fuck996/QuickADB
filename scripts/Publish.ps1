# Copyright (C) 2026 QuickADB contributors
# SPDX-License-Identifier: GPL-3.0-only
# 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

param(
    [string]$DestinationDirectory = 'U:\开发工作',
    [string]$Dumpbin
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$executable = Join-Path $projectRoot 'target\x86_64-pc-windows-msvc\release\QuickADB.exe'
if (!(Test-Path -LiteralPath $executable -PathType Leaf)) { throw '请先完成测试、Release 构建和窗口验收。' }
if (!$Dumpbin) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (!$installation) { throw '找不到 Visual Studio C++ Build Tools 的依赖检查工具。' }
    $versions = Get-ChildItem -LiteralPath (Join-Path $installation 'VC\Tools\MSVC') -Directory | Sort-Object Name -Descending
    $Dumpbin = Join-Path $versions[0].FullName 'bin\Hostx64\x64\dumpbin.exe'
}
$audit = & $Dumpbin /dependents $executable
if ($LASTEXITCODE -ne 0) { throw 'EXE 依赖检查失败。' }
$dependencies = @($audit | ForEach-Object {
    if ($_ -match '^\s+(\S+\.dll)\s*$') { $Matches[1].ToLowerInvariant() }
})
if ($dependencies.Count -eq 0) { throw '未能解析 EXE 导入表。' }
$systemLibraries = @(
    'kernel32.dll', 'user32.dll', 'oleaut32.dll', 'uiautomationcore.dll',
    'gdi32.dll', 'ole32.dll', 'ntdll.dll', 'combase.dll', 'advapi32.dll',
    'bcryptprimitives.dll', 'ws2_32.dll', 'bcrypt.dll', 'iphlpapi.dll',
    'comctl32.dll', 'crypt32.dll', 'shell32.dll', 'opengl32.dll',
    'imm32.dll', 'dwmapi.dll', 'uxtheme.dll'
)
foreach ($library in $dependencies) {
    if ($library -notin $systemLibraries -and $library -notlike 'api-ms-win-*.dll') {
        throw "检测到未批准的运行依赖：$library"
    }
}
$cargo = [System.IO.File]::ReadAllText((Join-Path $projectRoot 'Cargo.toml'))
if ($cargo -notmatch '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"') { throw '无法读取应用版本。' }
$version = $Matches[1]
$versionInfo = (Get-Item -LiteralPath $executable).VersionInfo
if ($versionInfo.FileVersion -ne $version -or $versionInfo.ProductVersion -ne $version) {
    throw "EXE 版本与项目版本 $version 不一致，请重新构建 Release。"
}
$signature = Get-AuthenticodeSignature -FilePath $executable
$fileName = "QuickADB-$version-x64.exe"
$artifacts = Join-Path $projectRoot 'artifacts'
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
$local = Join-Path $artifacts $fileName
$hash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
function Copy-VerifiedExecutable([string]$Target) {
    if (Test-Path -LiteralPath $Target) {
        if ((Get-FileHash -LiteralPath $Target -Algorithm SHA256).Hash -ne $hash) {
            throw "同版本文件已存在但内容不同，请更新应用版本后发布：$Target"
        }
    } else { Copy-Item -LiteralPath $executable -Destination $Target }
    if ((Get-FileHash -LiteralPath $Target -Algorithm SHA256).Hash -ne $hash) { throw "复制校验失败：$Target" }
}
Copy-VerifiedExecutable $local
if (!(Test-Path -LiteralPath $DestinationDirectory -PathType Container)) { throw "发布副本目录不可访问：$DestinationDirectory；本地产物已保留：$local" }
$shared = Join-Path $DestinationDirectory $fileName
Copy-VerifiedExecutable $shared
[pscustomobject]@{
    Version = $version
    Local = $local
    Shared = $shared
    SHA256 = $hash
    Bytes = (Get-Item -LiteralPath $local).Length
    SignatureStatus = $signature.Status.ToString()
    Publisher = if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { $null }
    SystemDependencies = $dependencies
} | ConvertTo-Json -Depth 3
