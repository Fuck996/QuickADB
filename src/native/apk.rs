// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

use anyhow::{Context, Result, bail, ensure};
use std::{collections::BTreeMap, fs::File, io::Read, path::PathBuf, time::SystemTime};

#[derive(Clone, Debug)]
pub struct Apk {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
    pub package: String,
    pub version: String,
    pub split: String,
}

impl Apk {
    pub fn read(path: PathBuf) -> Result<Self> {
        ensure!(
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("apk")),
            "只接受 APK 文件：{}",
            path.display()
        );
        let file = File::open(&path).with_context(|| format!("无法打开 {}", path.display()))?;
        let metadata = file.metadata()?;
        let mut archive = zip::ZipArchive::new(file).context("APK ZIP 结构损坏")?;
        let mut manifest = archive
            .by_name("AndroidManifest.xml")
            .context("文件不是有效的 APK，缺少 AndroidManifest.xml")?;
        ensure!(manifest.size() <= 16 * 1024 * 1024, "APK 清单过大");
        let mut bytes = Vec::new();
        manifest.read_to_end(&mut bytes)?;
        let attributes = manifest_attributes(&bytes)?;
        let package = attributes
            .get("package")
            .context("APK 清单缺少包名")?
            .clone();
        ensure!(
            !package.is_empty()
                && package
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_'),
            "APK 包名无效"
        );
        let version = format!(
            "{}:{}",
            attributes
                .get("versionCodeMajor")
                .map(String::as_str)
                .unwrap_or("0"),
            attributes
                .get("versionCode")
                .context("APK 清单缺少 versionCode")?
        );
        Ok(Self {
            path,
            size: metadata.len(),
            modified: metadata.modified()?,
            package,
            version,
            split: attributes.get("split").cloned().unwrap_or_default(),
        })
    }
}

pub fn validate_group(apks: &[Apk], split_group: bool) -> Result<()> {
    ensure!(!apks.is_empty(), "没有选择 APK");
    if !split_group {
        ensure!(
            apks.iter().all(|a| a.split.is_empty()),
            "包含拆分 APK，请通过“选择拆分 APK”将基础包与拆分包作为一组添加"
        );
        return Ok(());
    }
    ensure!(apks.len() >= 2, "拆分安装至少需要基础包和一个拆分包");
    let first = &apks[0];
    ensure!(
        apks.iter()
            .all(|a| a.package == first.package && a.version == first.version),
        "拆分 APK 的包名或版本不同，不能合并安装"
    );
    ensure!(
        apks.iter().filter(|a| a.split.is_empty()).count() == 1,
        "拆分 APK 必须包含且只包含一个基础包"
    );
    let mut names = std::collections::BTreeSet::new();
    ensure!(
        apks.iter().all(|a| names.insert(&a.split)),
        "拆分 APK 名称重复"
    );
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    let slice = bytes
        .get(offset..offset.checked_add(2).context("清单偏移溢出")?)
        .context("APK 清单截断")?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let slice = bytes
        .get(offset..offset.checked_add(4).context("清单偏移溢出")?)
        .context("APK 清单截断")?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn pool_string(strings: &[String], index: u32) -> Result<&str> {
    strings
        .get(index as usize)
        .map(String::as_str)
        .context("APK 清单字符串索引无效")
}

pub fn manifest_attributes(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    ensure!(
        u16_at(bytes, 0)? == 3 && u32_at(bytes, 4)? as usize == bytes.len(),
        "APK 二进制清单格式无效"
    );
    let mut offset = u16_at(bytes, 2)? as usize;
    ensure!(offset >= 8, "APK 清单头无效");
    let mut strings = Vec::new();
    while offset < bytes.len() {
        let kind = u16_at(bytes, offset)?;
        let header = u16_at(bytes, offset + 2)? as usize;
        let size = u32_at(bytes, offset + 4)? as usize;
        ensure!(
            header >= 8 && size >= header && size <= bytes.len() - offset,
            "APK 清单数据块损坏"
        );
        let chunk = &bytes[offset..offset + size];
        if kind == 1 {
            ensure!(header >= 28, "APK 字符串池头损坏");
            let count = u32_at(chunk, 8)? as usize;
            let utf8 = u32_at(chunk, 16)? & 0x100 != 0;
            let start = u32_at(chunk, 20)? as usize;
            ensure!(
                count <= (size - header) / 4 && start <= size && start >= header + count * 4,
                "APK 字符串池损坏"
            );
            for i in 0..count {
                let mut position = start
                    .checked_add(u32_at(chunk, header + i * 4)? as usize)
                    .context("APK 字符串偏移溢出")?;
                let text = if utf8 {
                    length8(chunk, &mut position)?;
                    let length = length8(chunk, &mut position)?;
                    let data = chunk
                        .get(position..position + length)
                        .context("APK 字符串截断")?;
                    ensure!(
                        chunk.get(position + length) == Some(&0),
                        "APK 字符串缺少结尾"
                    );
                    std::str::from_utf8(data)
                        .context("APK UTF-8 字符串无效")?
                        .to_owned()
                } else {
                    let first = u16_at(chunk, position)? as usize;
                    position += 2;
                    let length = if first & 0x8000 != 0 {
                        let second = u16_at(chunk, position)? as usize;
                        position += 2;
                        ((first & 0x7fff) << 16) | second
                    } else {
                        first
                    };
                    ensure!(
                        length <= (size.saturating_sub(position)) / 2,
                        "APK UTF-16 字符串截断"
                    );
                    let mut units = Vec::with_capacity(length);
                    for j in 0..length {
                        units.push(u16_at(chunk, position + j * 2)?);
                    }
                    ensure!(
                        u16_at(chunk, position + length * 2)? == 0,
                        "APK 字符串缺少结尾"
                    );
                    String::from_utf16(&units).context("APK UTF-16 字符串无效")?
                };
                strings.push(text);
            }
        } else if kind == 0x102 && pool_string(&strings, u32_at(chunk, 20)?)? == "manifest" {
            ensure!(header >= 16, "APK 清单节点头损坏");
            let start = 16 + u16_at(chunk, 24)? as usize;
            let stride = u16_at(chunk, 26)? as usize;
            let count = u16_at(chunk, 28)? as usize;
            ensure!(
                stride >= 20 && start <= size && count <= (size - start) / stride,
                "APK 清单属性表损坏"
            );
            let mut attributes = BTreeMap::new();
            for i in 0..count {
                let position = start + i * stride;
                let name = pool_string(&strings, u32_at(chunk, position + 4)?)?;
                let raw = u32_at(chunk, position + 8)?;
                let typed = u32_at(chunk, position + 16)?;
                let value = if raw != u32::MAX {
                    pool_string(&strings, raw)?.to_owned()
                } else if chunk[position + 15] == 3 {
                    pool_string(&strings, typed)?.to_owned()
                } else {
                    typed.to_string()
                };
                ensure!(
                    attributes.insert(name.into(), value).is_none(),
                    "APK 清单属性重复：{name}"
                );
            }
            return Ok(attributes);
        }
        offset += size;
    }
    bail!("APK 清单缺少 manifest 节点")
}

fn length8(bytes: &[u8], position: &mut usize) -> Result<usize> {
    let first = *bytes.get(*position).context("APK 字符串长度截断")?;
    *position += 1;
    if first & 0x80 == 0 {
        return Ok(first as usize);
    }
    let second = *bytes.get(*position).context("APK 字符串长度截断")?;
    *position += 1;
    Ok(((first as usize & 0x7f) << 8) | second as usize)
}
