// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

use super::{Release, select_release};
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::c_void,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{self, Command},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, WAIT_OBJECT_0},
    Networking::WinHttp::*,
    Security::Cryptography::{
        BCRYPT_SHA256_ALGORITHM, BCryptCloseAlgorithmProvider, BCryptHash,
        BCryptOpenAlgorithmProvider,
    },
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

const RELEASE_API: &str = "/repos/Fuck996/QuickADB/releases/latest";
const RESTART_ARG: &str = "--quickadb-update-restart";

pub fn check() -> Result<Option<Release>> {
    ensure!(cfg!(target_arch = "x86_64"), "自动更新仅支持 Windows x64");
    let body = https_get("api.github.com", RELEASE_API, 1024 * 1024)?;
    select_release(serde_json::from_slice(&body)?, env!("CARGO_PKG_VERSION"))
}

pub fn download(release: &Release) -> Result<Vec<u8>> {
    let bytes = https_get("github.com", &release.path, release.size)?;
    ensure!(
        bytes.len() == release.size && sha256(&bytes)? == release.digest,
        "更新文件大小或 SHA-256 校验失败"
    );
    Ok(bytes)
}

struct HttpHandle(*mut c_void);

impl HttpHandle {
    fn new(handle: *mut c_void) -> io::Result<Self> {
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }
}

impl Drop for HttpHandle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn win_bool(value: i32) -> io::Result<()> {
    if value == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn https_get(host: &str, path: &str, limit: usize) -> Result<Vec<u8>> {
    let agent = crate::platform::wide(concat!("QuickADB/", env!("CARGO_PKG_VERSION")));
    let host = crate::platform::wide(host);
    let path = crate::platform::wide(path);
    let started = Instant::now();
    // WinHTTP validates TLS certificates and by default blocks redirects to HTTP.
    let (_session, _connection, request) = unsafe {
        let session = HttpHandle::new(WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        ))?;
        win_bool(WinHttpSetTimeouts(
            session.0, 10_000, 10_000, 15_000, 15_000,
        ))?;
        let connection = HttpHandle::new(WinHttpConnect(session.0, host.as_ptr(), 443, 0))?;
        let request = HttpHandle::new(WinHttpOpenRequest(
            connection.0,
            null(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        win_bool(WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0))?;
        win_bool(WinHttpReceiveResponse(request.0, null_mut()))?;
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        win_bool(WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&raw mut status).cast(),
            &mut size,
            null_mut(),
        ))?;
        ensure!(status == 200, "GitHub 返回 HTTP {status}");
        (session, connection, request)
    };
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 32 * 1024];
    loop {
        ensure!(
            started.elapsed() < Duration::from_secs(180),
            "从 GitHub 下载超时"
        );
        let mut read = 0;
        unsafe {
            win_bool(WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            ))?;
        }
        if read == 0 {
            break;
        }
        ensure!(
            bytes.len() + read as usize <= limit,
            "GitHub 响应超过允许的大小"
        );
        bytes.extend_from_slice(&buffer[..read as usize]);
    }
    Ok(bytes)
}

fn sha256(bytes: &[u8]) -> Result<[u8; 32]> {
    let mut digest = [0u8; 32];
    unsafe {
        let mut algorithm = null_mut();
        let status =
            BCryptOpenAlgorithmProvider(&mut algorithm, BCRYPT_SHA256_ALGORITHM, null(), 0);
        ensure!(status >= 0, "无法初始化 SHA-256：{status:#x}");
        let status = BCryptHash(
            algorithm,
            null(),
            0,
            bytes.as_ptr(),
            bytes.len() as u32,
            digest.as_mut_ptr(),
            digest.len() as u32,
        );
        BCryptCloseAlgorithmProvider(algorithm, 0);
        ensure!(status >= 0, "无法计算 SHA-256：{status:#x}");
    }
    Ok(digest)
}

fn staging_directory(exe: &Path, pid: u32) -> PathBuf {
    let mut name = exe.as_os_str().to_os_string();
    name.push(format!(".update-{pid}"));
    PathBuf::from(name)
}

pub fn install(bytes: &[u8]) -> Result<()> {
    let exe = std::env::current_exe()?;
    let pid = process::id();
    let dir = staging_directory(&exe, pid);
    fs::create_dir(&dir).context("无法在 QuickADB 所在目录暂存更新，请确认该目录可写")?;
    let pending = dir.join("download.exe");
    let backup = dir.join("previous.exe");
    let result = (|| -> Result<()> {
        let mut file = File::create_new(&pending)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&exe, &backup).context("无法备份当前 QuickADB")?;
        if let Err(error) = fs::rename(&pending, &exe) {
            restore(&backup, &exe)?;
            return Err(error).context("无法替换 QuickADB");
        }
        if let Err(error) = Command::new(&exe)
            .arg(RESTART_ARG)
            .arg(pid.to_string())
            .spawn()
        {
            restore(&backup, &exe)?;
            return Err(error).context("新版 QuickADB 无法启动，已还原旧版");
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(pending);
        let _ = fs::remove_dir(dir);
    }
    result
}

fn restore(backup: &Path, exe: &Path) -> Result<()> {
    fs::rename(backup, exe)
        .with_context(|| format!("无法自动还原，旧版保留在 {}", backup.display()))
}

pub fn wait_for_previous_process() -> Result<Option<u32>> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(RESTART_ARG)) {
        return Ok(None);
    }
    let pid = args
        .next()
        .and_then(|s| s.to_str()?.parse::<u32>().ok())
        .filter(|id| *id != 0 && *id != process::id())
        .context("无效的更新重启参数")?;
    unsafe {
        let parent = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !parent.is_null() {
            let result = WaitForSingleObject(parent, 30_000);
            CloseHandle(parent);
            if result != WAIT_OBJECT_0 {
                bail!("旧版 QuickADB 尚未退出，请稍后重新打开");
            }
        }
    }
    Ok(Some(pid))
}

pub fn remove_backup(pid: u32) -> Result<()> {
    let dir = staging_directory(&std::env::current_exe()?, pid);
    fs::remove_file(dir.join("previous.exe"))?;
    fs::remove_dir(dir)?;
    Ok(())
}
