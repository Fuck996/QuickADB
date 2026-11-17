// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

use crate::{engine::Backend, model::*, storage::Storage};
use anyhow::{Context, Result, ensure};
use droidmux::discovery::AdbServiceType;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char},
    fs::{self, File},
    path::PathBuf,
    ptr,
    sync::Arc,
};

pub struct MacBackend {
    backend: Backend,
    storage: Arc<Storage>,
    _lock: File,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Command {
    Toggle {
        id: String,
    },
    SelectAll {
        selected: bool,
    },
    Prepare {
        paths: Vec<PathBuf>,
        split: bool,
    },
    ClearPreparation,
    Install {
        revision: u64,
    },
    RetryJob {
        job_id: u64,
    },
    Cancel {
        job_id: u64,
    },
    ClearFinished,
    ClearNotice,
    Pair {
        host: String,
        port: u16,
        code: String,
    },
    Connect {
        host: String,
        port: u16,
    },
    ConnectPaired {
        id: String,
        host: Option<String>,
        port: Option<u16>,
    },
    RetryConnection {
        id: String,
    },
    EditConnection {
        id: String,
        host: Option<String>,
        port: Option<u16>,
        paired_id: Option<String>,
    },
    EnableTcp {
        id: String,
        host: String,
        port: u16,
    },
    Disconnect {
        id: String,
    },
    Forget {
        id: String,
    },
    Alias {
        id: String,
        name: String,
    },
    Preferences {
        pinned: Option<bool>,
        test_packages: Option<bool>,
    },
}

fn endpoint(host: String, port: u16, paired_id: Option<String>) -> Result<Endpoint> {
    let host = host.trim().to_owned();
    ensure!(!host.is_empty() && port != 0, "请输入有效的地址和端口");
    Ok(Endpoint {
        host,
        port,
        paired_id,
    })
}

fn apply(app: &MacBackend, command: Command) -> Result<()> {
    let backend = &app.backend;
    match command {
        Command::Toggle { id } => backend.toggle(&id),
        Command::SelectAll { selected } => backend.select_all(selected),
        Command::Prepare { paths, split } => backend.prepare(paths, split),
        Command::ClearPreparation => backend.clear_preparation(),
        Command::Install { revision } => {
            backend.install_prepared(revision, app.storage.settings().test_packages)
        }
        Command::RetryJob { job_id } => backend.retry_job(job_id),
        Command::Cancel { job_id } => backend.cancel(job_id),
        Command::ClearFinished => backend.clear_finished(),
        Command::ClearNotice => backend.clear_notice(),
        Command::Pair { host, port, code } => {
            let address = endpoint(host, port, None)?;
            ensure!(
                code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()),
                "请输入手机显示的六位配对码"
            );
            backend.pair(address.host, address.port, code, None);
        }
        Command::Connect { host, port } => backend.connect(endpoint(host, port, None)?),
        Command::ConnectPaired { id, host, port } => match (host, port) {
            (Some(host), Some(port)) => backend.connect(endpoint(host, port, Some(id))?),
            (None, None) => backend.connect_paired(id),
            _ => anyhow::bail!("手动连接需要同时填写地址和端口"),
        },
        Command::RetryConnection { id } => backend.retry_connection(id),
        Command::EditConnection {
            id,
            host,
            port,
            paired_id,
        } => {
            let address = match (host, port) {
                (Some(host), Some(port)) => Some(endpoint(host, port, paired_id)?),
                (None, None) if id.starts_with("tls:") => None,
                _ => anyhow::bail!("手动连接需要同时填写地址和端口"),
            };
            backend.update_connection(id, address);
        }
        Command::EnableTcp { id, host, port } => {
            let address = endpoint(host, port, None)?;
            backend.enable_tcp(id, address.host, address.port);
        }
        Command::Disconnect { id } => backend.disconnect(id),
        Command::Forget { id } => backend.forget_device(id),
        Command::Alias { id, name } => {
            let snapshot = backend.snapshot();
            let device = snapshot
                .devices
                .iter()
                .find(|device| device.id == id)
                .context("设备记录已不存在")?;
            backend.set_alias(device, &name);
        }
        Command::Preferences {
            pinned,
            test_packages,
        } => app.storage.update_settings(|settings| {
            if let Some(value) = pinned {
                settings.pinned = value;
            }
            if let Some(value) = test_packages {
                settings.test_packages = value;
            }
        })?,
    }
    Ok(())
}

fn snapshot(app: &MacBackend) -> Value {
    let state = app.backend.snapshot();
    let settings = app.storage.settings();
    let devices: Vec<_> = state.devices.iter().map(|device| json!({
        "id": device.id, "name": device.name, "model": device.model, "serial": device.serial,
        "android": device.android, "transport": device.transport, "status": device.status.label(),
        "online": device.status == DeviceStatus::Online, "selected": device.selected,
        "selectable": device.selectable(), "managing": state.managing.contains(&device.id),
        "detail": device.detail, "endpoint": device.endpoint,
        "paired_id": device.id.strip_prefix("tls:"),
        "alias": settings.device_alias(device.alias_key(), &device.serial),
    })).collect();
    let jobs: Vec<_> = state
        .jobs
        .iter()
        .map(|job| {
            let stage = match job.stage {
                JobStage::Transferring => "transferring",
                JobStage::Installing => "installing",
                JobStage::Succeeded => "succeeded",
                JobStage::Canceled => "canceled",
                JobStage::Failed | JobStage::Unknown => "failed",
                _ => "waiting",
            };
            json!({
                "id": job.id, "title": job.title, "device": job.device_name,
                "status": job.stage.label(), "stage": stage, "active": job.stage.active(),
                "transferred": job.transferred, "total": job.total, "speed": job.bytes_per_second,
                "detail": job.detail,
            })
        })
        .collect();
    let preparation = state.preparation.map(|item| {
        let (status, apks, error) = match item.state {
            PreparationState::Checking => ("checking", Vec::new(), None),
            PreparationState::Ready(apks) => ("ready", apks, None),
            PreparationState::Failed(error) => ("failed", Vec::new(), Some(error)),
        };
        let files: Vec<_> = item.paths.iter().enumerate().map(|(index, path)| {
            let apk = apks.get(index);
            json!({
                "path": path.to_string_lossy(),
                "size": apk.map(|apk| apk.size),
                "package": apk.map(|apk| &apk.package),
                "split": apk.map(|apk| &apk.split),
            })
        }).collect();
        json!({"revision": item.revision, "split": item.split, "status": status, "files": files, "error": error})
    });
    let discovered: Vec<_> = state.discovered.iter().map(|item| {
        let kind = match item.service_type {
            AdbServiceType::Pairing => "pairing",
            AdbServiceType::TlsConnect => "tls",
            AdbServiceType::Legacy => "tcp",
        };
        json!({
            "id": item.fullname, "name": item.name, "kind": kind, "endpoint": item.endpoint,
            "paired_id": item.paired_id().filter(|id| state.devices.iter().any(|device| device.id.strip_prefix("tls:") == Some(id))),
        })
    }).collect();
    json!({
        "devices": devices, "jobs": jobs, "preparation": preparation, "discovered": discovered,
        "notice": state.notice, "connecting": state.connecting, "pairing": state.pairing,
        "pinned": settings.pinned, "test_packages": settings.test_packages,
    })
}

fn c_string(text: &str) -> *mut c_char {
    CString::new(text.replace('\0', "\\0"))
        .expect("NUL characters replaced")
        .into_raw()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn quickadb_open(
    directory: *const c_char,
    error: *mut *mut c_char,
) -> *mut MacBackend {
    let open = || -> Result<MacBackend> {
        let directory = PathBuf::from(unsafe { CStr::from_ptr(directory) }.to_str()?);
        fs::create_dir_all(&directory).context("无法创建应用数据目录")?;
        let lock = File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("instance.lock"))
            .context("无法打开应用锁")?;
        lock.try_lock()
            .context("无法获取应用锁，请检查 QuickADB 是否已运行")?;
        let storage = Storage::open(directory)?;
        let backend = Backend::new(storage.clone())?;
        Ok(MacBackend {
            backend,
            storage,
            _lock: lock,
        })
    };
    match open() {
        Ok(app) => Box::into_raw(Box::new(app)),
        Err(reason) => {
            unsafe { *error = c_string(&format!("{reason:#}")) };
            ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn quickadb_snapshot(app: *const MacBackend) -> *mut c_char {
    c_string(&snapshot(unsafe { &*app }).to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn quickadb_command(
    app: *const MacBackend,
    command: *const c_char,
) -> *mut c_char {
    let payload = unsafe { CStr::from_ptr(command) }.to_bytes();
    let result = serde_json::from_slice(payload)
        .context("无法读取应用命令")
        .and_then(|command| apply(unsafe { &*app }, command));
    match result {
        Ok(()) => ptr::null_mut(),
        Err(reason) => c_string(&format!("{reason:#}")),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn quickadb_close(app: *mut MacBackend) {
    drop(unsafe { Box::from_raw(app) });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn quickadb_string_free(text: *mut c_char) {
    drop(unsafe { CString::from_raw(text) });
}
