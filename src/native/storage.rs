// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

use crate::model::{Endpoint, Settings};
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use droidmux::{
    auth::RsaAdbCredential,
    pairing::{CredentialStore, CredentialStoreError, StoredHostCredential, StoredPairedDevice},
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};

#[derive(Serialize, Deserialize)]
struct HostKey {
    pem: String,
    comment: String,
}

pub struct Storage {
    pub directory: PathBuf,
    settings: Mutex<Settings>,
    paired: Mutex<BTreeMap<String, PairedRecord>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PairedRecord {
    id: String,
    host: String,
    fingerprint: String,
}

impl Storage {
    pub fn open(directory: PathBuf) -> Result<Arc<Self>> {
        fs::create_dir_all(&directory).context("无法创建应用数据目录")?;
        let settings = read_json::<Settings>(&directory.join("settings.json"))?.unwrap_or_default();
        let paired = read_json::<BTreeMap<String, PairedRecord>>(&directory.join("paired.json"))?
            .unwrap_or_default();
        let result = Arc::new(Self {
            directory,
            settings: Mutex::new(settings),
            paired: Mutex::new(paired),
        });
        if !result.directory.join("host.key").exists() {
            let credential = RsaAdbCredential::generate("QuickADB")?;
            let pem = credential.to_pkcs8_pem()?;
            result.write_key(&pem, "QuickADB")?;
        }
        result.credential()?;
        Ok(result)
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .lock()
            .expect("settings lock poisoned")
            .clone()
    }

    pub fn paired_devices(&self) -> Vec<StoredPairedDevice> {
        self.paired
            .lock()
            .expect("pairing lock poisoned")
            .values()
            .map(|d| StoredPairedDevice {
                device_id: d.id.clone(),
                host: d.host.clone(),
                certificate_fingerprint: d.fingerprint.clone(),
            })
            .collect()
    }

    pub fn update_settings(&self, change: impl FnOnce(&mut Settings)) -> Result<()> {
        self.change_settings(|settings| {
            change(settings);
            Ok(())
        })
    }

    fn change_settings(&self, change: impl FnOnce(&mut Settings) -> Result<()>) -> Result<()> {
        let mut saved = self.settings.lock().expect("settings lock poisoned");
        let mut updated = saved.clone();
        change(&mut updated)?;
        write_json(&self.directory.join("settings.json"), &updated)?;
        *saved = updated;
        Ok(())
    }

    pub fn replace_connection(&self, id: &str, endpoint: Option<&Endpoint>) -> Result<()> {
        self.change_settings(|settings| {
            let new_id = endpoint.map_or_else(|| id.to_owned(), Endpoint::key);
            ensure!(
                new_id == id || !settings.endpoints.iter().any(|e| e.key() == new_id),
                "此连接地址已有设备记录，请编辑已有记录"
            );
            settings.endpoints.retain(|e| e.key() != id);
            if let Some(endpoint) = endpoint {
                settings.endpoints.push(endpoint.clone());
            }
            if new_id != id {
                if let Some(alias) = settings.aliases.remove(id) {
                    settings.aliases.insert(new_id.clone(), alias);
                }
                if let Some(info) = settings.device_info.remove(id) {
                    settings.device_info.insert(new_id, info);
                }
            }
            Ok(())
        })
    }

    pub fn forget_wireless(&self, id: &str) -> Result<()> {
        let mut settings = self.settings.lock().expect("settings lock poisoned");
        let mut paired = self.paired.lock().expect("pairing lock poisoned");
        let mut updated_settings = settings.clone();
        updated_settings.endpoints.retain(|e| e.key() != id);
        updated_settings.aliases.remove(id);
        updated_settings.device_info.remove(id);
        let mut updated_paired = paired.clone();
        let removed = id
            .strip_prefix("tls:")
            .and_then(|key| updated_paired.remove(key))
            .is_some();
        let pairing_path = self.directory.join("paired.json");
        if removed {
            write_json(&pairing_path, &updated_paired)?;
        }
        if let Err(error) = write_json(&self.directory.join("settings.json"), &updated_settings) {
            if removed && let Err(rollback) = write_json(&pairing_path, &*paired) {
                *paired = updated_paired;
                anyhow::bail!(
                    "配对记录已删除，但连接配置清理失败：{error:#}；恢复配对记录也失败：{rollback:#}。请修复数据目录权限后重试删除"
                );
            }
            return Err(error);
        }
        *settings = updated_settings;
        *paired = updated_paired;
        Ok(())
    }

    pub fn credential(&self) -> Result<Arc<RsaAdbCredential>> {
        let key = self.read_key()?;
        Ok(Arc::new(RsaAdbCredential::from_pkcs8_pem(
            &key.pem,
            &key.comment,
        )?))
    }

    fn read_key(&self) -> Result<HostKey> {
        let encrypted =
            fs::read(self.directory.join("host.key")).context("无法读取设备授权密钥")?;
        let decrypted = protect(&encrypted, false)
            .context("无法解密设备授权密钥，请使用创建它的 Windows 账号")?;
        serde_json::from_slice(&decrypted).context("设备授权密钥格式损坏")
    }

    fn write_key(&self, pem: &str, comment: &str) -> Result<()> {
        let bytes = serde_json::to_vec(&HostKey {
            pem: pem.into(),
            comment: comment.into(),
        })?;
        let encrypted = protect(&bytes, true).context("无法保护设备授权密钥")?;
        atomic_write(&self.directory.join("host.key"), &encrypted)
    }

    pub fn log(&self, message: &str) -> Result<()> {
        use std::io::Write;
        let path = self.directory.join("quickadb.log");
        if fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
            let previous = self.directory.join("quickadb.previous.log");
            if previous.exists() {
                fs::remove_file(&previous)?;
            }
            fs::rename(&path, previous)?;
        }
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        writeln!(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?,
            "{timestamp} {message}"
        )?;
        Ok(())
    }
}

#[async_trait]
impl CredentialStore for Storage {
    async fn load_host_credential(
        &self,
    ) -> Result<Option<StoredHostCredential>, CredentialStoreError> {
        let key = self.read_key().map_err(store_error)?;
        Ok(Some(StoredHostCredential::new(
            SecretString::from(key.pem),
            key.comment,
        )))
    }
    async fn save_host_credential(
        &self,
        credential: &StoredHostCredential,
    ) -> Result<(), CredentialStoreError> {
        self.write_key(
            credential.expose_private_key_pem(),
            credential.client_name(),
        )
        .map_err(store_error)
    }
    async fn save_paired_device(
        &self,
        device: &StoredPairedDevice,
    ) -> Result<(), CredentialStoreError> {
        let mut records = self.paired.lock().expect("pairing lock poisoned");
        let mut updated = records.clone();
        updated.insert(
            device.device_id.clone(),
            PairedRecord {
                id: device.device_id.clone(),
                host: device.host.clone(),
                fingerprint: device.certificate_fingerprint.clone(),
            },
        );
        write_json(&self.directory.join("paired.json"), &updated).map_err(store_error)?;
        *records = updated;
        Ok(())
    }
    async fn load_paired_device(
        &self,
        id: &str,
    ) -> Result<Option<StoredPairedDevice>, CredentialStoreError> {
        Ok(self
            .paired
            .lock()
            .expect("pairing lock poisoned")
            .get(id)
            .map(|d| StoredPairedDevice {
                device_id: d.id.clone(),
                host: d.host.clone(),
                certificate_fingerprint: d.fingerprint.clone(),
            }))
    }
}

fn store_error(error: anyhow::Error) -> CredentialStoreError {
    CredentialStoreError::new(format!("授权数据保存或读取失败：{error:#}"))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => {
            Ok(Some(serde_json::from_slice(&bytes).with_context(|| {
                format!("配置文件损坏：{}", path.display())
            })?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("无法读取配置：{}", path.display())),
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path).with_context(|| format!("无法保存 {}", path.display()))
}

fn protect(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len())?,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // DPAPI 返回的缓冲区由 LocalFree 释放；此处不保存未加密的私钥文件。
    unsafe {
        let success = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if success == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(result)
    }
}
