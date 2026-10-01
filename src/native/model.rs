use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub startup: bool,
    pub pinned: bool,
    pub topmost: bool,
    pub dark: bool,
    pub test_packages: bool,
    pub window_position: Option<[f32; 2]>,
    pub aliases: BTreeMap<String, String>,
    pub endpoints: Vec<Endpoint>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobStage {
    Queued,
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
            Self::Queued | Self::Preparing | Self::Transferring | Self::Installing
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Queued => "排队中",
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
    pub connecting: bool,
    pub pairing: bool,
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
