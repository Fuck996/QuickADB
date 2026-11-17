// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{check, download, install, remove_backup, wait_for_previous_process};

const REPOSITORY: &str = "https://github.com/Fuck996/QuickADB";
const MAX_EXE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone)]
pub struct Release {
    pub version: String,
    pub page: String,
    path: String,
    size: usize,
    digest: [u8; 32],
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
    size: usize,
    digest: Option<String>,
}

fn version_parts(version: &str) -> Option<[u32; 3]> {
    let mut segments = version.split('.');
    let mut parts = [0; 3];
    for part in &mut parts {
        let segment = segments.next()?;
        if !segment.bytes().all(|b| b.is_ascii_digit())
            || (segment.starts_with('0') && segment.len() > 1)
        {
            return None;
        }
        *part = segment.parse().ok()?;
    }
    segments.next().is_none().then_some(parts)
}

fn select_release(metadata: GitHubRelease, current: &str) -> Result<Option<Release>> {
    let tag = &metadata.tag_name;
    if metadata.draft || metadata.prerelease {
        return Ok(None);
    }
    let version = tag.strip_prefix('v').context("发布标签缺少版本前缀")?;
    let published = version_parts(version).context("发布标签不是受支持的正式版版本号")?;
    let installed = version_parts(current).context("当前版本不是受支持的正式版版本号")?;
    if published <= installed {
        return Ok(None);
    }
    let name = format!("QuickADB-{version}-x64.exe");
    let asset = metadata
        .assets
        .iter()
        .find(|item| item.name == name && item.state == "uploaded")
        .context("该版本未提供 Windows x64 更新文件")?;
    ensure!(
        (1..=MAX_EXE_BYTES).contains(&asset.size),
        "发布文件大小超出限制"
    );
    let sha = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
        .context("GitHub 未提供有效的发布文件 SHA-256")?;
    let mut digest = [0; 32];
    for (i, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&sha[i * 2..i * 2 + 2], 16)?;
    }
    Ok(Some(Release {
        version: version.into(),
        page: format!("{REPOSITORY}/releases/tag/{tag}"),
        path: format!("/Fuck996/QuickADB/releases/download/{tag}/{name}"),
        size: asset.size,
        digest,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(
        tag: &str,
        prerelease: bool,
        asset_name: &str,
        digest: Option<String>,
    ) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            assets: vec![Asset {
                name: asset_name.into(),
                state: "uploaded".into(),
                size: 123,
                digest,
            }],
        }
    }

    #[test]
    fn compares_numeric_versions_and_requires_the_exact_windows_asset() -> Result<()> {
        let sha = format!("sha256:{}", "ab".repeat(32));
        let name = "QuickADB-0.9.10-x64.exe";
        let release = select_release(metadata("v0.9.10", false, name, Some(sha.clone())), "0.9.9")?
            .context("数值更大的版本应可更新")?;
        assert_eq!(release.version, "0.9.10");
        assert_eq!(release.page, format!("{REPOSITORY}/releases/tag/v0.9.10"));
        assert_eq!(
            release.path,
            "/Fuck996/QuickADB/releases/download/v0.9.10/QuickADB-0.9.10-x64.exe"
        );
        assert_eq!(release.size, 123);
        assert_eq!(release.digest, [0xab; 32]);
        assert!(
            select_release(
                metadata("v0.9.10", false, name, Some(sha.clone())),
                "0.9.10"
            )?
            .is_none()
        );
        assert!(
            select_release(metadata("v0.9.10", false, name, Some(sha.clone())), "1.0.0")?.is_none()
        );
        assert!(
            select_release(
                metadata("v0.9.10", false, "QuickADB-0.9.10-arm64.dmg", Some(sha)),
                "0.9.9"
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn skips_prereleases_and_rejects_unverifiable_assets() -> Result<()> {
        let beta = metadata("v0.9.10-mac-beta.1", true, "QuickADB-arm64.dmg", None);
        assert!(select_release(beta, "0.9.9")?.is_none());
        let asset_without_digest = metadata("v0.9.10", false, "QuickADB-0.9.10-x64.exe", None);
        assert!(select_release(asset_without_digest, "0.9.9").is_err());
        Ok(())
    }
}
