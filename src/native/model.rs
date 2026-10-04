use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub startup: bool,
    pub pinned: bool,
    pub topmost: bool,
    pub dark: bool,
    pub test_packages: bool,
    pub window_position: Option<[f32; 2]>,
    pub windowed: bool,
    pub standard_window_size: Option<[f32; 2]>,
    pub standard_window_position: Option<[f32; 2]>,
    pub aliases: BTreeMap<String, String>,
    pub endpoints: Vec<Endpoint>,
    pub device_info: BTreeMap<String, DeviceInfo>,
}

impl Settings {
    pub fn device_alias(&self, id: &str, serial: &str) -> Option<&str> {
        // 旧版备注使用序列号；无线记录改用固定设备 ID，空值表示用户已清除备注。
        self.aliases
            .get(id)
            .or_else(|| self.aliases.get(serial))
            .filter(|alias| !alias.is_empty())
            .map(String::as_str)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub model: String,
    pub serial: String,
    pub android: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub paired_id: Option<String>,
}

impl Endpoint {
    pub fn key(&self) -> String {
        match &self.paired_id {
            Some(id) => format!("tls:{id}"),
            None => format!("tcp://{}:{}", self.host, self.port),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceStatus {
    Connecting,
    Unauthorized,
    Online,
    Offline,
    Disconnected,
}

impl DeviceStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Connecting => "连接中",
            Self::Unauthorized => "等待手机授权",
            Self::Online => "已连接",
            Self::Offline => "离线",
            Self::Disconnected => "已断开",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: String,
    pub serial: String,
    pub android: String,
    pub transport: String,
    pub status: DeviceStatus,
    pub detail: String,
    pub selected: bool,
    pub endpoint: Option<Endpoint>,
}

impl Device {
    pub fn alias_key(&self) -> &str {
        if self.endpoint.is_some() || self.id.starts_with("tls:") {
            &self.id
        } else {
            &self.serial
        }
    }

    pub fn selectable(&self) -> bool {
        self.status == DeviceStatus::Online
            || self.endpoint.is_some()
            || self.id.starts_with("tls:")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobStage {
    Queued,
    Connecting,
    Preparing,
    Transferring,
    Installing,
    Succeeded,
    Failed,
    Canceled,
    Unknown,
}

impl JobStage {
    pub fn active(&self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Connecting
                | Self::Preparing
                | Self::Transferring
                | Self::Installing
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Queued => "排队中",
            Self::Connecting => "正在连接设备…",
            Self::Preparing => "准备安装",
            Self::Transferring => "正在传输",
            Self::Installing => "正在安装…",
            Self::Succeeded => "安装成功",
            Self::Failed => "安装失败",
            Self::Canceled => "已取消",
            Self::Unknown => "结果未知，请在手机上确认",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: u64,
    pub device_id: String,
    pub device_name: String,
    pub paths: Vec<PathBuf>,
    pub title: String,
    pub stage: JobStage,
    pub transferred: u64,
    pub total: u64,
    pub bytes_per_second: f64,
    pub detail: String,
    pub test_packages: bool,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub devices: Vec<Device>,
    pub jobs: Vec<Job>,
    pub preparation: Option<ApkPreparation>,
    pub discovered: Vec<DiscoveredDevice>,
    pub notice: String,
    pub(crate) notice_deadline: Option<Instant>,
    pub connecting: bool,
    pub pairing: bool,
}

impl Snapshot {
    pub(crate) fn set_notice(&mut self, message: String) {
        self.notice = message;
        self.notice_deadline = Some(Instant::now() + Duration::from_secs(8));
    }

    pub(crate) fn clear_notice(&mut self) {
        self.notice.clear();
        self.notice_deadline = None;
    }

    pub(crate) fn expire_notice(&mut self) {
        if self
            .notice_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.clear_notice();
        }
    }
}

#[derive(Clone, Debug)]
pub struct ApkPreparation {
    pub revision: u64,
    pub paths: Vec<PathBuf>,
    pub split: bool,
    pub state: PreparationState,
}

#[derive(Clone, Debug)]
pub enum PreparationState {
    Checking,
    Ready(Vec<crate::apk::Apk>),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct DiscoveredDevice {
    pub fullname: String,
    pub instance_name: String,
    pub name: String,
    pub model_advertised: bool,
    pub service_type: droidmux::discovery::AdbServiceType,
    pub endpoint: Endpoint,
}

impl DiscoveredDevice {
    pub fn paired_id(&self) -> Option<&str> {
        (self.service_type == droidmux::discovery::AdbServiceType::TlsConnect)
            .then_some(self.instance_name.as_str())
    }
}
