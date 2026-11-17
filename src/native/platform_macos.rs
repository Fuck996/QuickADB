// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn data_directory() -> Result<PathBuf> {
    let home = std::env::home_dir().context("无法定位 macOS 用户目录")?;
    Ok(home.join("Library/Application Support/QuickADB"))
}

#[doc(hidden)]
pub fn remove_fixture_credential(directory: &Path) -> Result<()> {
    crate::keychain::delete(directory)
}
