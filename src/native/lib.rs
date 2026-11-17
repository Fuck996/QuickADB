// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

pub mod apk;
pub mod engine;
pub mod install;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(any(target_os = "macos", test))]
mod mac_bridge;
pub mod model;
#[cfg(target_os = "windows")]
pub mod platform;
#[cfg(target_os = "macos")]
#[path = "platform_macos.rs"]
pub mod platform;
pub mod storage;
#[cfg(any(target_os = "windows", test))]
pub mod update;
