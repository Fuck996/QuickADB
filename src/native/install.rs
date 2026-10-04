// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

use crate::{apk::Apk, model::JobStage};
use anyhow::{Context, Result, bail, ensure};
use droidmux::client::{AdbClient, AdbStream};
use std::{
    os::windows::fs::OpenOptionsExt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Progress {
    pub stage: JobStage,
    pub bytes: u64,
    pub total: u64,
    pub speed: f64,
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub struct InstallError {
    pub stage: JobStage,
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Rejected(String);

#[derive(Debug, thiserror::Error)]
#[error("用户取消安装")]
struct Canceled;

pub async fn install(
    client: Arc<AdbClient>,
    apks: &[Apk],
    test_packages: bool,
    cancel: CancellationToken,
    report: impl Fn(Progress) + Send + Sync,
) -> std::result::Result<(), InstallError> {
    let total = apks.iter().map(|p| p.size).sum();
    let may_have_committed = AtomicBool::new(false);
    let operation = Operation {
        client: &client,
        total,
        start: Instant::now(),
        report: &report,
        cancel: &cancel,
        may_have_committed: &may_have_committed,
    };
    operation.progress(JobStage::Preparing, 0);
    let result = if client.supports_feature("cmd") || client.supports_feature("abb_exec") {
        operation.stream_install(apks, test_packages).await
    } else {
        operation.legacy_install(apks, test_packages).await
    };
    result.map_err(|error| {
        let stage = if error.downcast_ref::<Rejected>().is_some() {
            JobStage::Failed
        } else if may_have_committed.load(Ordering::Acquire) {
            JobStage::Unknown
        } else if error.downcast_ref::<Canceled>().is_some() {
            JobStage::Canceled
        } else {
            JobStage::Failed
        };
        InstallError {
            stage,
            detail: format!("{error:#}"),
        }
    })
}

struct Operation<'a, F> {
    client: &'a AdbClient,
    total: u64,
    start: Instant,
    report: &'a F,
    cancel: &'a CancellationToken,
    may_have_committed: &'a AtomicBool,
}

impl<F: Fn(Progress) + Send + Sync> Operation<'_, F> {
    fn progress(&self, stage: JobStage, bytes: u64) {
        (self.report)(Progress {
            stage,
            bytes,
            total: self.total,
            speed: bytes as f64 / self.start.elapsed().as_secs_f64().max(0.001),
            detail: String::new(),
        });
    }

    async fn open_package(&self, args: &[String]) -> Result<AdbStream> {
        if self.client.supports_feature("abb_exec") {
            let mut arguments = vec!["package"];
            arguments.extend(args.iter().map(String::as_str));
            Ok(tokio::time::timeout(
                Duration::from_secs(30),
                self.client.open_abb_exec_args(&arguments),
            )
            .await
            .context("打开安装服务超时")??)
        } else {
            Ok(tokio::time::timeout(
                Duration::from_secs(30),
                self.client
                    .open_service(&format!("exec:cmd package {}", args.join(" "))),
            )
            .await
            .context("打开安装服务超时")??)
        }
    }

    async fn package_result(&self, args: &[String]) -> Result<String> {
        let stream = self.open_package(args).await?;
        output(&stream).await
    }

    async fn stream_install(&self, apks: &[Apk], test: bool) -> Result<()> {
        let mut flags = vec!["-r".to_string()];
        if test {
            flags.push("-t".into());
        }
        if apks.len() == 1 {
            let args = [
                vec!["install".into()],
                flags,
                vec!["-S".into(), self.total.to_string()],
            ]
            .concat();
            let stream = self.open_package(&args).await?;
            self.send(&stream, &apks[0], 0, true, false).await?;
            check_success(&output(&stream).await?)?;
            return Ok(());
        }
        let args = [
            vec!["install-create".into()],
            flags,
            vec!["-S".into(), self.total.to_string()],
        ]
        .concat();
        let created = self.package_result(&args).await?;
        check_success(&created)?;
        let session = session_id(&created)?;
        let result = async {
            let mut sent = 0;
            for (index, apk) in apks.iter().enumerate() {
                let args = vec![
                    "install-write".into(),
                    "-S".into(),
                    apk.size.to_string(),
                    session.clone(),
                    format!("part{index}.apk"),
                    "-".into(),
                ];
                let stream = self.open_package(&args).await?;
                self.send(&stream, apk, sent, false, false).await?;
                check_success(&output(&stream).await?)?;
                sent += apk.size;
            }
            if self.cancel.is_cancelled() {
                return Err(Canceled.into());
            }
            self.may_have_committed.store(true, Ordering::Release);
            self.progress(JobStage::Installing, self.total);
            check_success(
                &self
                    .package_result(&["install-commit".into(), session.clone()])
                    .await?,
            )
        }
        .await;
        if let Err(error) = result {
            let cleanup = self
                .package_result(&["install-abandon".into(), session.clone()])
                .await
                .and_then(|output| check_success(&output));
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => {
                    Err(error.context(format!("安装会话 {session} 清理失败：{cleanup:#}")))
                }
            };
        }
        Ok(())
    }

    async fn send(
        &self,
        stream: &AdbStream,
        apk: &Apk,
        previous: u64,
        last_commits: bool,
        sync: bool,
    ) -> Result<()> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&apk.path)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.len() == apk.size && metadata.modified()? == apk.modified,
            "APK 在加入队列后发生变化，请重新添加"
        );
        let mut file = tokio::fs::File::from_std(file);
        let mut buffer = vec![
            0;
            stream
                .max_payload()
                .min(if sync { 64 * 1024 } else { 256 * 1024 })
        ];
        let mut sent = 0;
        let mut last_report = Instant::now();
        self.progress(JobStage::Transferring, previous);
        loop {
            if self.cancel.is_cancelled() && !self.may_have_committed.load(Ordering::Acquire) {
                return Err(Canceled.into());
            }
            let count = file.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            if last_commits && sent + count as u64 == apk.size {
                // 单包最后一块到达后 Android 可能立即提交，此后只能等待真实结果。
                self.may_have_committed.store(true, Ordering::Release);
            }
            let write = async {
                if sync {
                    write_chunks(stream, &sync_message(b"DATA", &buffer[..count])).await
                } else {
                    stream
                        .write(buffer[..count].to_vec())
                        .await
                        .map_err(anyhow::Error::from)
                }
            };
            if self.may_have_committed.load(Ordering::Acquire) {
                tokio::time::timeout(Duration::from_secs(30), write)
                    .await
                    .context("APK 传输超时")??;
            } else {
                tokio::select! {
                    _ = self.cancel.cancelled() => return Err(Canceled.into()),
                    result = tokio::time::timeout(Duration::from_secs(30), write) => result.context("APK 传输超时")??,
                }
            }
            sent += count as u64;
            if last_report.elapsed() >= Duration::from_millis(80) || sent == apk.size {
                self.progress(JobStage::Transferring, previous + sent);
                last_report = Instant::now();
            }
        }
        ensure!(sent == apk.size, "APK 读取字节数与文件大小不同");
        if last_commits {
            self.progress(JobStage::Installing, self.total);
        }
        Ok(())
    }

    async fn legacy_install(&self, apks: &[Apk], test: bool) -> Result<()> {
        ensure!(apks.len() == 1, "此设备不支持拆分 APK 安装");
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let remote = format!("/data/local/tmp/quickadb-{suffix}.apk");
        let result = async {
            let stream = self.client.open_service("sync:").await?;
            write_chunks(
                &stream,
                &sync_message(b"SEND", format!("{remote},33188").as_bytes()),
            )
            .await?;
            self.send(&stream, &apks[0], 0, false, true).await?;
            let mut done = b"DONE".to_vec();
            let mtime = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs() as u32;
            done.extend_from_slice(&mtime.to_le_bytes());
            write_chunks(&stream, &done).await?;
            let mut response = Vec::new();
            while response.len() < 8 {
                response.extend_from_slice(
                    &tokio::time::timeout(Duration::from_secs(30), stream.read())
                        .await??
                        .context("设备未确认 APK 传输")?,
                );
            }
            if &response[..4] == b"FAIL" {
                let length = u32::from_le_bytes(response[4..8].try_into()?) as usize;
                ensure!(length <= 65536, "设备文件同步错误消息过大");
                while response.len() < 8 + length {
                    response.extend_from_slice(&stream.read().await?.context("错误消息截断")?);
                }
                bail!("{}", String::from_utf8_lossy(&response[8..8 + length]));
            }
            ensure!(&response[..4] == b"OKAY", "设备文件同步没有确认成功");
            stream.close().await?;
            if self.cancel.is_cancelled() {
                return Err(Canceled.into());
            }
            self.may_have_committed.store(true, Ordering::Release);
            self.progress(JobStage::Installing, self.total);
            let command = format!("pm install -r {} {remote}", if test { "-t" } else { "" });
            let result = crate::engine::shell_text_with_timeout(
                self.client,
                &command,
                Duration::from_secs(600),
            )
            .await?;
            check_success(&result)
        }
        .await;
        let cleanup = crate::engine::shell_text(self.client, &format!("rm -f {remote}")).await;
        match (result, cleanup) {
            (Ok(()), Ok(output)) if output.trim().is_empty() => Ok(()),
            (Ok(()), cleanup) => {
                (self.report)(Progress {
                    stage: JobStage::Succeeded,
                    bytes: self.total,
                    total: self.total,
                    speed: 0.,
                    detail: format!("APK 已安装，但临时文件清理失败：{cleanup:?}"),
                });
                Ok(())
            }
            (Err(error), Ok(output)) if output.trim().is_empty() => Err(error),
            (Err(error), cleanup) => {
                Err(error.context(format!("设备临时文件清理失败：{cleanup:?}")))
            }
        }
    }
}

async fn write_chunks(stream: &AdbStream, bytes: &[u8]) -> Result<()> {
    for chunk in bytes.chunks(stream.max_payload()) {
        stream.write(chunk.to_vec()).await?;
    }
    Ok(())
}

fn sync_message(command: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut message = command.to_vec();
    message.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    message.extend_from_slice(payload);
    message
}

async fn output(stream: &AdbStream) -> Result<String> {
    tokio::time::timeout(Duration::from_secs(600), async {
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.read().await? {
            ensure!(bytes.len() + chunk.len() <= 65536, "安装服务响应超过限制");
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).context("安装服务响应不是 UTF-8")
    })
    .await
    .context("等待 Android 安装结果超时，请在手机上确认")?
}

fn check_success(output: &str) -> Result<()> {
    let output = output.trim();
    if output == "Success" || output.starts_with("Success:") {
        return Ok(());
    }
    if output.is_empty() {
        bail!("设备未返回安装结果");
    }
    Err(Rejected(output.into()).into())
}

fn session_id(output: &str) -> Result<String> {
    let (_, remaining) = output.split_once('[').context("设备未返回安装会话 ID")?;
    let (id, _) = remaining.split_once(']').context("设备安装会话 ID 截断")?;
    ensure!(
        !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()),
        "设备安装会话 ID 无效"
    );
    Ok(id.into())
}
