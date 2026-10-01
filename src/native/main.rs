#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ui;

fn main() {
    if let Err(error) = run() { quickadb::platform::show_error(&format!("{error:#}")); }
}

fn run() -> anyhow::Result<()> {
    let Some(_instance) = quickadb::platform::Instance::acquire()? else { return Ok(()); };
    let storage = quickadb::storage::Storage::open(quickadb::platform::data_directory()?)?;
    let backend = quickadb::engine::Backend::new(storage.clone())?;
    ui::run(storage, backend).map_err(|error| anyhow::anyhow!("窗口初始化失败：{error}"))
}
