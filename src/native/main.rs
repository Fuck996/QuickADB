// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ui;

fn main() {
    if let Err(error) = run() {
        quickadb::platform::show_error(&format!("{error:#}"));
    }
}

fn run() -> anyhow::Result<()> {
    let Some(_instance) = quickadb::platform::Instance::acquire()? else {
        return Ok(());
    };
    let storage = quickadb::storage::Storage::open(quickadb::platform::data_directory()?)?;
    if storage.settings().startup {
        quickadb::platform::set_startup(true)?;
    }
    let backend = quickadb::engine::Backend::new(storage.clone())?;
    ui::run(storage, backend).map_err(|error| anyhow::anyhow!("窗口初始化失败：{error}"))
}
