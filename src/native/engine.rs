// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

use crate::{
    apk::{Apk, validate_group},
    install,
    model::*,
    storage::Storage,
};
use anyhow::{Context, Result, ensure};
use droidmux::{
    auth::RsaAdbCredential,
    client::{AdbClient, ConnectionState},
    pairing::{PairingRequest, WirelessTransport, WirelessTransportConfig, pair_device},
    tcp::{TcpTransport, TcpTransportConfig},
    usb::{UsbDeviceInfo, UsbTransport, UsbTransportConfig},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    runtime::Runtime,
    sync::{Mutex as AsyncMutex, mpsc},
};
use tokio_util::sync::CancellationToken;

pub struct Backend {
    runtime: Option<Runtime>,
    engine: Arc<Engine>,
    submissions: mpsc::UnboundedSender<Submission>,
}

struct Submission {
    packages: Packages,
    split: bool,
    test: bool,
    targets: Vec<Device>,
}

enum Packages {
    Paths(Vec<PathBuf>),
    Prepared(Vec<Apk>),
}

struct QueueItem {
    job: Job,
    apks: Vec<Apk>,
    target: Device,
    cancel: CancellationToken,
}

enum WirelessTarget {
    Paired(String),
    Address(Endpoint),
}

struct Engine {
    state: Mutex<Snapshot>,
    storage: Arc<Storage>,
    credential: Arc<RsaAdbCredential>,
    clients: AsyncMutex<BTreeMap<String, Arc<AdbClient>>>,
    usb: Mutex<BTreeMap<String, UsbDeviceInfo>>,
    connecting: Mutex<BTreeSet<String>>,
    connection_locks: AsyncMutex<BTreeMap<String, Arc<AsyncMutex<()>>>>,
    lanes: AsyncMutex<BTreeMap<String, mpsc::UnboundedSender<QueueItem>>>,
    cancellations: Mutex<BTreeMap<u64, CancellationToken>>,
    next_job: AtomicU64,
    next_preparation: AtomicU64,
    stop: CancellationToken,
}

impl Backend {
    pub fn new(storage: Arc<Storage>) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(3)
            .build()?;
        let devices = saved_devices(&storage);
        let engine = Arc::new(Engine {
            credential: storage.credential()?,
            storage,
            state: Mutex::new(Snapshot {
                devices,
                ..Default::default()
            }),
            clients: AsyncMutex::new(BTreeMap::new()),
            usb: Mutex::new(BTreeMap::new()),
            connecting: Mutex::new(BTreeSet::new()),
            connection_locks: AsyncMutex::new(BTreeMap::new()),
            lanes: AsyncMutex::new(BTreeMap::new()),
            cancellations: Mutex::new(BTreeMap::new()),
            next_job: AtomicU64::new(1),
            next_preparation: AtomicU64::new(1),
            stop: CancellationToken::new(),
        });
        runtime.spawn(engine.clone().monitor());
        let (submissions, mut pending) = mpsc::unbounded_channel::<Submission>();
        let submit_engine = engine.clone();
        runtime.spawn(async move {
            while let Some(request) = pending.recv().await {
                submit_engine
                    .clone()
                    .submit(
                        request.packages,
                        request.split,
                        request.test,
                        request.targets,
                    )
                    .await;
            }
        });
        for endpoint in engine.storage.settings().endpoints {
            if endpoint.paired_id.is_none() {
                runtime.spawn(engine.clone().connect_endpoint(endpoint));
            }
        }
        Ok(Self {
            runtime: Some(runtime),
            engine,
            submissions,
        })
    }

    fn spawn(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        self.runtime
            .as_ref()
            .expect("backend runtime missing")
            .spawn(task);
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut state = self.engine.state.lock().expect("state lock poisoned");
        state.expire_notice();
        state.clone()
    }

    pub fn toggle(&self, id: &str) {
        if let Some(device) = self
            .engine
            .state
            .lock()
            .expect("state lock poisoned")
            .devices
            .iter_mut()
            .find(|d| d.id == id && d.selectable())
        {
            device.selected = !device.selected;
        }
    }

    pub fn select_all(&self, selected: bool) {
        for device in &mut self
            .engine
            .state
            .lock()
            .expect("state lock poisoned")
            .devices
        {
            if device.selectable() {
                device.selected = selected;
            }
        }
    }

    pub fn connect(&self, endpoint: Endpoint) {
        self.spawn(self.engine.clone().connect_endpoint(endpoint));
    }

    pub fn pair(
        &self,
        host: String,
        pairing_port: u16,
        code: String,
        connection_port: Option<u16>,
    ) {
        let engine = self.engine.clone();
        self.spawn(async move {
            engine.state.lock().expect("state lock poisoned").pairing = true;
            let result = pair_device(
                PairingRequest {
                    host: host.clone(),
                    port: pairing_port,
                    pairing_code: code.into(),
                    client_name: "QuickADB".into(),
                },
                engine.storage.as_ref(),
            )
            .await;
            engine.state.lock().expect("state lock poisoned").pairing = false;
            match result {
                Ok(result) => {
                    {
                        let mut state = engine.state.lock().expect("state lock poisoned");
                        for device in saved_devices(&engine.storage) {
                            if !state.devices.iter().any(|known| known.id == device.id) {
                                state.devices.push(device);
                            }
                        }
                    }
                    engine.notice("配对成功，正在查找设备的无线连接服务");
                    match connection_port {
                        Some(port) => {
                            engine
                                .connect_endpoint(Endpoint {
                                    host,
                                    port,
                                    paired_id: Some(result.device_id),
                                })
                                .await
                        }
                        None => engine.connect_paired(result.device_id).await,
                    }
                }
                Err(error) => engine.notice(&format!("无线配对失败：{error}")),
            }
        });
    }

    pub fn connect_paired(&self, device_id: String) {
        self.spawn(self.engine.clone().connect_paired(device_id));
    }

    pub fn retry_connection(&self, id: String) {
        if let Some(device_id) = id.strip_prefix("tls:") {
            self.connect_paired(device_id.to_owned());
            return;
        }
        let endpoint = self
            .snapshot()
            .devices
            .iter()
            .find(|d| d.id == id)
            .and_then(|d| d.endpoint.clone());
        if let Some(endpoint) = endpoint {
            match &endpoint.paired_id {
                Some(device_id) => self.connect_paired(device_id.clone()),
                None => self.connect(endpoint),
            }
        } else {
            let info = self
                .engine
                .usb
                .lock()
                .expect("usb lock poisoned")
                .get(&id)
                .cloned();
            if let Some(info) = info {
                self.spawn(self.engine.clone().connect_usb(id, info));
            }
        }
    }

    pub fn disconnect(&self, id: String) {
        let engine = self.engine.clone();
        self.spawn(async move {
            let active = engine
                .state
                .lock()
                .expect("state lock poisoned")
                .jobs
                .iter()
                .any(|j| j.device_id == id && j.stage.active());
            if active {
                engine.notice("设备仍有安装任务，请先等待任务结束或取消传输");
                return;
            }
            if let Some(client) = engine.clients.lock().await.remove(&id)
                && let Err(error) = client.close().await
            {
                engine.notice(&format!("断开设备时发生错误：{error}"));
            }
            engine.update_device(&id, |d| {
                d.status = DeviceStatus::Disconnected;
                d.selected = false;
            });
        });
    }

    pub fn enable_tcp(&self, id: String, host: String, port: u16) {
        let engine = self.engine.clone();
        self.spawn(async move {
            let client = engine.clients.lock().await.get(&id).cloned();
            let Some(client) = client else {
                engine.notice("USB 设备已断开");
                return;
            };
            if engine
                .state
                .lock()
                .expect("state lock poisoned")
                .jobs
                .iter()
                .any(|j| j.device_id == id && j.stage.active())
            {
                engine.notice("请等待此设备的安装任务结束后再切换无线调试");
                return;
            }
            match tokio::time::timeout(Duration::from_secs(15), client.tcpip(port)).await {
                Ok(Ok(output)) => {
                    let output = String::from_utf8_lossy(&output);
                    if !output.contains("restarting in TCP mode") {
                        engine.notice(&format!("手机未确认切换 TCP 调试：{output}"));
                        return;
                    }
                    engine.clients.lock().await.remove(&id);
                    engine.update_device(&id, |d| {
                        d.status = DeviceStatus::Disconnected;
                        d.selected = false;
                    });
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    engine
                        .connect_endpoint(Endpoint {
                            host,
                            port,
                            paired_id: None,
                        })
                        .await;
                }
                Ok(Err(error)) => engine.notice(&format!("USB 转无线失败：{error}")),
                Err(error) => engine.notice(&format!("USB 转无线超时：{error}")),
            }
        });
    }

    pub fn prepare(&self, paths: Vec<PathBuf>, split: bool) {
        if paths.is_empty() {
            self.clear_preparation();
            return;
        }
        let revision = self.engine.next_preparation.fetch_add(1, Ordering::Relaxed);
        self.engine
            .state
            .lock()
            .expect("state lock poisoned")
            .preparation = Some(ApkPreparation {
            revision,
            paths: paths.clone(),
            split,
            state: PreparationState::Checking,
        });
        let engine = self.engine.clone();
        self.spawn(async move {
            let result = read_apks(paths, split).await;
            let mut state = engine.state.lock().expect("state lock poisoned");
            if let Some(preparation) = &mut state.preparation
                && preparation.revision == revision
            {
                preparation.state = match result {
                    Ok(apks) => PreparationState::Ready(apks),
                    Err(error) => PreparationState::Failed(format!("{error:#}")),
                };
            }
        });
    }

    pub fn clear_preparation(&self) {
        self.engine
            .state
            .lock()
            .expect("state lock poisoned")
            .preparation = None;
    }

    pub fn install_prepared(&self, revision: u64, test: bool) {
        let mut state = self.engine.state.lock().expect("state lock poisoned");
        let Some(preparation) = &state.preparation else {
            return;
        };
        if preparation.revision != revision {
            return;
        }
        let PreparationState::Ready(apks) = &preparation.state else {
            return;
        };
        let targets: Vec<_> = state
            .devices
            .iter()
            .filter(|d| d.selected && d.selectable())
            .cloned()
            .collect();
        if targets.is_empty() {
            drop(state);
            self.engine
                .notice("请先点击选择至少一台设备，再点击安装；离线无线设备会尝试重连");
            return;
        }
        if self
            .submissions
            .send(Submission {
                packages: Packages::Prepared(apks.clone()),
                split: preparation.split,
                test,
                targets,
            })
            .is_err()
        {
            drop(state);
            self.engine.notice("APK 提交队列已关闭");
        } else {
            state.preparation = None;
        }
    }

    pub fn submit(&self, paths: Vec<PathBuf>, split: bool, test: bool) {
        let targets: Vec<_> = self
            .snapshot()
            .devices
            .into_iter()
            .filter(|d| d.selected && d.selectable())
            .collect();
        if targets.is_empty() {
            self.engine
                .notice("请先点击选择至少一台设备，再点击安装；离线无线设备会尝试重连");
            return;
        }
        if self
            .submissions
            .send(Submission {
                packages: Packages::Paths(paths),
                split,
                test,
                targets,
            })
            .is_err()
        {
            self.engine.notice("APK 提交队列已关闭");
        }
    }

    pub fn retry_job(&self, id: u64) {
        let snapshot = self.snapshot();
        let Some(job) = snapshot
            .jobs
            .iter()
            .find(|j| j.id == id && !j.stage.active())
        else {
            return;
        };
        let Some(device) = snapshot
            .devices
            .iter()
            .find(|d| d.id == job.device_id && d.selectable())
        else {
            self.engine.notice("原目标设备未连接，请连接原设备后重试");
            return;
        };
        if self
            .submissions
            .send(Submission {
                packages: Packages::Paths(job.paths.clone()),
                split: job.paths.len() > 1,
                test: job.test_packages,
                targets: vec![device.clone()],
            })
            .is_err()
        {
            self.engine.notice("APK 提交队列已关闭");
        }
    }

    pub fn cancel(&self, id: u64) {
        let snapshot = self.snapshot();
        if snapshot
            .jobs
            .iter()
            .any(|j| j.id == id && j.stage == JobStage::Installing)
        {
            self.engine.notice("Android 正在提交安装，请等待真实结果");
            return;
        }
        if let Some(token) = self
            .engine
            .cancellations
            .lock()
            .expect("cancellation lock poisoned")
            .get(&id)
        {
            token.cancel();
        }
    }

    pub fn clear_finished(&self) {
        self.engine
            .state
            .lock()
            .expect("state lock poisoned")
            .jobs
            .retain(|j| j.stage.active());
    }

    pub fn clear_notice(&self) {
        self.engine
            .state
            .lock()
            .expect("state lock poisoned")
            .clear_notice();
    }

    pub fn notify(&self, message: &str) {
        self.engine.notice(message);
    }

    pub fn set_alias(&self, device: &Device, alias: &str) {
        let key = device.alias_key();
        if key.is_empty() {
            self.engine.notice("尚未读取 USB 设备序列号，无法保存备注");
            return;
        }
        match self.engine.storage.update_settings(|settings| {
            if key == device.serial && alias.trim().is_empty() {
                settings.aliases.remove(key);
            } else {
                settings.aliases.insert(key.into(), alias.trim().into());
            }
        }) {
            Ok(()) => self.engine.update_device(&device.id, |d| {
                d.name = if alias.trim().is_empty() {
                    d.model.clone()
                } else {
                    alias.trim().into()
                }
            }),
            Err(error) => self.engine.notice(&format!("设备备注保存失败：{error:#}")),
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.engine.stop.cancel();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(3));
        }
    }
}

impl Engine {
    fn notice(&self, message: &str) {
        let log = self.storage.log(message);
        let message = match log {
            Ok(()) => message.to_owned(),
            Err(error) => format!("{message}\n日志保存失败：{error:#}"),
        };
        self.state
            .lock()
            .expect("state lock poisoned")
            .set_notice(message);
    }

    fn update_device(&self, id: &str, action: impl FnOnce(&mut Device)) {
        if let Some(device) = self
            .state
            .lock()
            .expect("state lock poisoned")
            .devices
            .iter_mut()
            .find(|d| d.id == id)
        {
            action(device);
        }
    }

    fn start_connect(&self, id: &str, name: &str, endpoint: Option<Endpoint>) -> bool {
        if !self
            .connecting
            .lock()
            .expect("connecting lock poisoned")
            .insert(id.to_owned())
        {
            return false;
        }
        let mut state = self.state.lock().expect("state lock poisoned");
        if let Some(device) = state.devices.iter_mut().find(|d| d.id == id) {
            device.status = DeviceStatus::Connecting;
            device.detail.clear();
            device.endpoint = endpoint;
        } else {
            state.devices.push(Device {
                id: id.into(),
                name: name.into(),
                model: name.into(),
                serial: String::new(),
                android: String::new(),
                transport: if endpoint.is_some() {
                    "无线".into()
                } else {
                    "USB".into()
                },
                status: DeviceStatus::Connecting,
                detail: String::new(),
                selected: false,
                endpoint,
            });
        }
        true
    }

    async fn connect_usb(self: Arc<Self>, id: String, info: UsbDeviceInfo) {
        if !self.start_connect(&id, &info.description, None) {
            return;
        }
        let result = async {
            let transport = UsbTransport::connect_device(
                &info,
                UsbTransportConfig {
                    read_timeout: Duration::from_secs(660),
                    ..Default::default()
                },
            )
            .await
            .context("无法打开 USB 调试接口。请检查驱动，以及其他 ADB 工具是否占用此接口")?;
            let observer_engine = self.clone();
            let observer_id = id.clone();
            let client = tokio::time::timeout(
                Duration::from_secs(120),
                AdbClient::connect_with_observer(
                    Box::new(transport),
                    self.credential.clone(),
                    move |state| {
                        if state == ConnectionState::AwaitingAuthorization {
                            observer_engine.update_device(&observer_id, |d| {
                                d.status = DeviceStatus::Unauthorized
                            });
                        }
                    },
                ),
            )
            .await
            .context("等待 USB 授权超时，请在手机上确认调试授权后重试")??;
            Ok::<_, anyhow::Error>(client)
        }
        .await;
        let _ = self.finish_connect(id, result, &self.stop).await;
    }

    async fn connect_paired(self: Arc<Self>, device_id: String) {
        if let Err(error) = self
            .clone()
            .connect_wireless(WirelessTarget::Paired(device_id), &self.stop)
            .await
        {
            self.notice(&format!("无线连接失败：{error:#}"));
        }
    }

    async fn connect_endpoint(self: Arc<Self>, endpoint: Endpoint) {
        if let Err(error) = self
            .clone()
            .connect_wireless(WirelessTarget::Address(endpoint), &self.stop)
            .await
        {
            self.notice(&format!("无线连接失败：{error:#}"));
        }
    }

    async fn connect_wireless(
        self: Arc<Self>,
        target: WirelessTarget,
        cancel: &CancellationToken,
    ) -> Result<Arc<AdbClient>> {
        let (id, endpoint) = match &target {
            WirelessTarget::Paired(device_id) => {
                ensure!(
                    self.storage
                        .paired_devices()
                        .iter()
                        .any(|d| d.device_id == *device_id),
                    "此无线设备尚未与 QuickADB 配对"
                );
                let id = format!("tls:{device_id}");
                let endpoint = self
                    .storage
                    .settings()
                    .endpoints
                    .into_iter()
                    .find(|e| e.key() == id);
                (id, endpoint)
            }
            WirelessTarget::Address(endpoint) => (endpoint.key(), Some(endpoint.clone())),
        };
        let gate = self
            .connection_locks
            .lock()
            .await
            .entry(id.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _connection = tokio::select! {
            _ = cancel.cancelled() => anyhow::bail!("连接已取消"),
            _ = self.stop.cancelled() => anyhow::bail!("应用正在退出"),
            guard = gate.lock() => guard,
        };
        {
            let mut clients = self.clients.lock().await;
            if let Some(client) = clients.get(&id) {
                if client.state() != ConnectionState::Closed {
                    return Ok(client.clone());
                }
                clients.remove(&id);
            }
        }
        ensure!(
            self.start_connect(&id, &id, endpoint),
            "设备连接请求正在处理中"
        );
        let attempt = async {
            match target {
                WirelessTarget::Address(endpoint) => self.dial_endpoint(&id, endpoint).await,
                WirelessTarget::Paired(device_id) => {
                    self.update_device(&id, |d| d.detail = "正在查找当前无线连接端口…".into());
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
                    let mut last_error = None;
                    loop {
                        let endpoint = self
                            .state
                            .lock()
                            .expect("state lock poisoned")
                            .discovered
                            .iter()
                            .find(|d| d.paired_id() == Some(device_id.as_str()))
                            .map(|d| Endpoint {
                                paired_id: Some(device_id.clone()),
                                ..d.endpoint.clone()
                            });
                        if let Some(endpoint) = endpoint {
                            self.update_device(&id, |d| d.endpoint = Some(endpoint.clone()));
                            match self.dial_endpoint(&id, endpoint).await {
                                Ok(client) => return Ok(client),
                                Err(error) => {
                                    self.update_device(&id, |d| {
                                        d.detail = format!("等待无线连接恢复：{error:#}")
                                    });
                                    last_error = Some(error);
                                }
                            }
                        }
                        if tokio::time::Instant::now() >= deadline {
                            return Err(match last_error {
                                Some(error) => error.context("无线重连失败"),
                                None => anyhow::anyhow!(
                                    "尚未发现此设备的无线连接服务。请开启手机无线调试并确认与电脑在同一局域网；配对记录仍保留。"
                                ),
                            });
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        };
        let result = tokio::select! {
            _ = cancel.cancelled() => Err(anyhow::anyhow!("连接已取消")),
            _ = self.stop.cancelled() => Err(anyhow::anyhow!("应用正在退出")),
            result = tokio::time::timeout(Duration::from_secs(30), attempt) => {
                result.context("无线连接超时，请检查手机无线调试与网络").and_then(|result| result)
            },
        };
        self.finish_connect(id, result, cancel).await
    }

    async fn dial_endpoint(self: &Arc<Self>, id: &str, endpoint: Endpoint) -> Result<AdbClient> {
        ensure!(
            !endpoint.host.trim().is_empty() && endpoint.port > 0,
            "设备地址或端口无效"
        );
        let transport: Box<dyn droidmux::transport::AdbTransport> =
            if let Some(device_id) = &endpoint.paired_id {
                use droidmux::pairing::CredentialStore;
                ensure!(
                    self.storage.load_paired_device(device_id).await?.is_some(),
                    "此无线设备尚未与 QuickADB 配对"
                );
                Box::new(
                    WirelessTransport::connect(
                        &endpoint.host,
                        endpoint.port,
                        &self.credential,
                        WirelessTransportConfig {
                            read_timeout: Duration::from_secs(660),
                            ..Default::default()
                        },
                    )
                    .await?,
                )
            } else {
                let address = tokio::net::lookup_host((endpoint.host.as_str(), endpoint.port))
                    .await?
                    .next()
                    .context("设备地址未解析到可连接地址")?;
                Box::new(
                    TcpTransport::connect(
                        address,
                        TcpTransportConfig {
                            read_timeout: Duration::from_secs(660),
                            ..Default::default()
                        },
                    )
                    .await?,
                )
            };
        let engine = self.clone();
        let observer_id = id.to_owned();
        let client = tokio::time::timeout(
            Duration::from_secs(120),
            AdbClient::connect_with_observer(transport, self.credential.clone(), move |state| {
                if state == ConnectionState::AwaitingAuthorization {
                    engine.update_device(&observer_id, |d| d.status = DeviceStatus::Unauthorized);
                }
            }),
        )
        .await
        .context("等待无线设备授权超时")??;
        Ok(client)
    }

    async fn finish_connect(
        self: &Arc<Self>,
        id: String,
        result: Result<AdbClient>,
        cancel: &CancellationToken,
    ) -> Result<Arc<AdbClient>> {
        let result = match result {
            Ok(client) => {
                let info = tokio::select! {
                    _ = cancel.cancelled() => Err(anyhow::anyhow!("连接已取消")),
                    result = shell_text(&client, "getprop ro.product.model && getprop ro.serialno && getprop ro.build.version.release") => result,
                };
                let info = info.and_then(|output| {
                    let mut info = parse_device_info(&output)?;
                    let settings = self.storage.settings();
                    let saved = settings.device_info.get(&id);
                    if let Some(expected) = saved.map(|d| &d.serial).filter(|s| !s.is_empty()) {
                        ensure!(
                            &info.serial == expected,
                            "连接到的设备序列号与原记录不同，已停止连接与安装"
                        );
                    }
                    let mut missing = Vec::new();
                    if info.model.is_empty() {
                        missing.push("型号");
                        if let Some(saved) = saved {
                            info.model.clone_from(&saved.model);
                        }
                    }
                    if info.android.is_empty() {
                        missing.push("Android 版本");
                        if let Some(saved) = saved {
                            info.android.clone_from(&saved.android);
                        }
                    }
                    let detail = if missing.is_empty() {
                        String::new()
                    } else {
                        format!("手机未返回{}；保留已保存的设备信息", missing.join("、"))
                    };
                    if id.starts_with("tls:") || id.starts_with("tcp://") {
                        let endpoint = self
                            .state
                            .lock()
                            .expect("state lock poisoned")
                            .devices
                            .iter()
                            .find(|d| d.id == id)
                            .and_then(|d| d.endpoint.clone());
                        self.storage.update_settings(|settings| {
                            if let Some(endpoint) = endpoint {
                                settings.endpoints.retain(|e| e.key() != id);
                                settings.endpoints.push(endpoint);
                            }
                            if !settings.aliases.contains_key(&id)
                                && let Some(alias) = settings.aliases.get(&info.serial).cloned()
                            {
                                settings.aliases.insert(id.clone(), alias);
                            }
                            settings.device_info.insert(id.clone(), info.clone());
                        })?;
                    }
                    Ok((info, detail))
                });
                match info {
                    Ok((info, detail)) => {
                        let alias = self
                            .storage
                            .settings()
                            .device_alias(&id, &info.serial)
                            .map(str::to_owned);
                        self.update_device(&id, |d| {
                            if !info.model.is_empty() {
                                d.model.clone_from(&info.model);
                            }
                            d.name = alias.unwrap_or_else(|| d.model.clone());
                            d.serial = info.serial;
                            d.android = info.android;
                            d.status = DeviceStatus::Online;
                            d.detail.clone_from(&detail);
                        });
                        if !detail.is_empty() {
                            self.notice(&format!("设备 {id}：{detail}"));
                        }
                        let client = Arc::new(client);
                        self.clients.lock().await.insert(id.clone(), client.clone());
                        tokio::spawn(self.clone().watch_connection(id.clone(), client.clone()));
                        Ok(client)
                    }
                    Err(error) => {
                        self.update_device(&id, |d| {
                            d.status = DeviceStatus::Offline;
                            d.detail = format!("读取设备信息失败：{error:#}");
                        });
                        if let Err(error) = client.close().await {
                            self.notice(&format!("读取设备信息失败后关闭连接失败：{error}"));
                        }
                        Err(error)
                    }
                }
            }
            Err(error) => {
                self.update_device(&id, |d| {
                    d.status = DeviceStatus::Offline;
                    d.detail = format!("{error:#}");
                });
                Err(error)
            }
        };
        self.connecting
            .lock()
            .expect("connecting lock poisoned")
            .remove(&id);
        result
    }

    async fn watch_connection(self: Arc<Self>, id: String, client: Arc<AdbClient>) {
        let mut last_check_error: Option<(String, String)> = None;
        loop {
            let reason = tokio::select! {
                _ = self.stop.cancelled() => return,
                reason = client.wait_closed() => Some(reason),
                _ = tokio::time::sleep(Duration::from_secs(10)) => None,
            };
            let result = if reason.is_none() {
                tokio::select! {
                    _ = self.stop.cancelled() => return,
                    result = shell_text(&client, "echo quickadb") => Some(result.and_then(|output| {
                        ensure!(output.trim() == "quickadb", "连接检查返回内容异常：{output:?}");
                        Ok(())
                    })),
                }
            } else {
                None
            };
            let mut clients = self.clients.lock().await;
            if !clients.get(&id).is_some_and(|c| Arc::ptr_eq(c, &client)) {
                return;
            }
            if reason.is_some() || client.state() == ConnectionState::Closed {
                let reason = match reason {
                    Some(reason) => reason,
                    None => client.wait_closed().await,
                };
                clients.remove(&id);
                self.update_device(&id, |d| {
                    d.status = DeviceStatus::Offline;
                    if d.endpoint.is_none() && !d.id.starts_with("tls:") {
                        d.selected = false;
                    }
                    d.detail = format!("设备已离线：{reason}");
                });
                drop(clients);
                // 会话离线是设备状态；安装中的失败由任务流程记录，避免重复全局告警。
                if let Err(error) = self.storage.log(&format!("设备 {id} 已离线：{reason}")) {
                    self.notice(&format!("设备离线记录保存失败：{error:#}"));
                }
                return;
            }
            match result {
                Some(Err(error)) => {
                    let key = match error.downcast_ref::<droidmux::shell::ShellError>() {
                        Some(droidmux::shell::ShellError::Client(
                            droidmux::client::AdbClientError::StreamRejected { .. },
                        )) => "ADB peer rejected connection check".into(),
                        _ => format!("{error:#}"),
                    };
                    let detail = format!("后台连接检查未完成：{error:#}");
                    self.update_device(&id, |d| d.detail.clone_from(&detail));
                    drop(clients);
                    if last_check_error.as_ref().map(|(key, _)| key) != Some(&key)
                        && let Err(error) = self.storage.log(&format!("设备 {id} {detail}"))
                    {
                        self.notice(&format!("连接检查日志保存失败：{error:#}"));
                    }
                    last_check_error = Some((key, detail));
                }
                Some(Ok(())) => {
                    if let Some((_, previous)) = last_check_error.take() {
                        self.update_device(&id, |d| {
                            if d.detail == previous {
                                d.detail.clear();
                            }
                        });
                        drop(clients);
                        if let Err(error) =
                            self.storage.log(&format!("设备 {id} 后台连接检查已恢复"))
                        {
                            self.notice(&format!("连接检查日志保存失败：{error:#}"));
                        }
                    }
                }
                None => {}
            }
        }
    }

    async fn monitor(self: Arc<Self>) {
        let mut previous_usb = BTreeSet::new();
        let discovery = match droidmux::discovery::MdnsDiscovery::new()
            .and_then(|d| d.browse().map(|r| (d, r)))
        {
            Ok(pair) => Some(pair),
            Err(error) => {
                self.notice(&format!("局域网自动发现不可用：{error}"));
                None
            }
        };
        loop {
            let result = tokio::task::spawn_blocking(UsbTransport::discover).await;
            match result {
                Ok(Ok(devices)) => {
                    let mut current = BTreeSet::new();
                    for info in devices {
                        let id = format!(
                            "usb:{:04x}:{:04x}:{}:{}",
                            info.vendor_id, info.product_id, info.bus_number, info.address
                        );
                        current.insert(id.clone());
                        self.usb
                            .lock()
                            .expect("usb lock poisoned")
                            .insert(id.clone(), info.clone());
                        if !previous_usb.contains(&id) {
                            tokio::spawn(self.clone().connect_usb(id, info));
                        }
                    }
                    for id in previous_usb.difference(&current) {
                        self.usb.lock().expect("usb lock poisoned").remove(id);
                        if let Some(client) = self.clients.lock().await.remove(id)
                            && let Err(error) = client.close().await
                        {
                            self.notice(&format!("USB 拔出后关闭连接：{error}"));
                        }
                        self.update_device(id, |d| {
                            d.status = DeviceStatus::Disconnected;
                            d.selected = false;
                            d.detail = "USB 设备已拔出".into();
                        });
                    }
                    previous_usb = current;
                }
                Ok(Err(error)) => self.notice(&format!("USB 设备枚举失败：{error}")),
                Err(error) => self.notice(&format!("USB 枚举任务失败：{error}")),
            }
            if let Some((_, receiver)) = &discovery {
                while let Ok(event) = receiver.try_recv() {
                    let device = match event {
                        droidmux::discovery::AdbMdnsEvent::Resolved(device) => device,
                        droidmux::discovery::AdbMdnsEvent::Removed(fullname) => {
                            self.state
                                .lock()
                                .expect("state lock poisoned")
                                .discovered
                                .retain(|device| device.fullname != fullname);
                            continue;
                        }
                    };
                    let addresses = device.ipv4_addresses();
                    if let Some(address) = addresses.first() {
                        let endpoint = Endpoint {
                            host: address.to_string(),
                            port: device.port,
                            paired_id: None,
                        };
                        let discovered = DiscoveredDevice {
                            fullname: device.fullname,
                            model_advertised: device.model.is_some(),
                            name: device
                                .model
                                .unwrap_or_else(|| format!("服务：{}", device.instance_name)),
                            instance_name: device.instance_name,
                            service_type: device.service_type,
                            endpoint,
                        };
                        let paired_id = discovered.paired_id().map(str::to_owned);
                        let connect = {
                            let mut state = self.state.lock().expect("state lock poisoned");
                            let changed = if let Some(existing) = state
                                .discovered
                                .iter_mut()
                                .find(|entry| entry.fullname == discovered.fullname)
                            {
                                let changed = existing.endpoint != discovered.endpoint;
                                *existing = discovered.clone();
                                changed
                            } else {
                                state.discovered.push(discovered.clone());
                                true
                            };
                            changed
                                && paired_id.as_ref().is_some_and(|id| {
                                    !state.devices.iter().any(|d| {
                                        d.id == format!("tls:{id}")
                                            && matches!(
                                                d.status,
                                                DeviceStatus::Online
                                                    | DeviceStatus::Connecting
                                                    | DeviceStatus::Unauthorized
                                                    | DeviceStatus::Disconnected
                                            )
                                    })
                                })
                        };
                        if connect
                            && let Some(id) = paired_id
                            && self
                                .storage
                                .paired_devices()
                                .iter()
                                .any(|d| d.device_id == id)
                        {
                            tokio::spawn(self.clone().connect_endpoint(Endpoint {
                                paired_id: Some(id),
                                ..discovered.endpoint
                            }));
                        }
                    }
                }
            }
            tokio::select! { _ = self.stop.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(2)) => {} }
        }
    }

    async fn submit(
        self: Arc<Self>,
        packages: Packages,
        split: bool,
        test: bool,
        targets: Vec<Device>,
    ) {
        let apks = match packages {
            Packages::Prepared(apks) => apks,
            Packages::Paths(paths) => match read_apks(paths, split).await {
                Ok(apks) => apks,
                Err(error) => {
                    self.notice(&format!("无法添加 APK：{error:#}"));
                    return;
                }
            },
        };
        let groups = if split {
            vec![apks]
        } else {
            apks.into_iter().map(|p| vec![p]).collect()
        };
        for target in targets {
            let lane_key = if target.serial.is_empty() {
                target.id.clone()
            } else {
                target.serial.clone()
            };
            let mut lanes = self.lanes.lock().await;
            let sender = lanes.entry(lane_key).or_insert_with(|| {
                let (sender, receiver) = mpsc::unbounded_channel();
                tokio::spawn(self.clone().run_lane(receiver));
                sender
            });
            for group in &groups {
                let id = self.next_job.fetch_add(1, Ordering::Relaxed);
                let title = if split {
                    format!("{} · {} 个拆分包", group[0].package, group.len())
                } else {
                    group[0]
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| group[0].package.clone())
                };
                let job = Job {
                    id,
                    device_id: target.id.clone(),
                    device_name: target.name.clone(),
                    paths: group.iter().map(|p| p.path.clone()).collect(),
                    title,
                    stage: JobStage::Queued,
                    transferred: 0,
                    total: group.iter().map(|p| p.size).sum(),
                    bytes_per_second: 0.,
                    detail: String::new(),
                    test_packages: test,
                };
                let cancel = CancellationToken::new();
                self.cancellations
                    .lock()
                    .expect("cancellation lock poisoned")
                    .insert(id, cancel.clone());
                self.state
                    .lock()
                    .expect("state lock poisoned")
                    .jobs
                    .push(job.clone());
                if sender
                    .send(QueueItem {
                        job,
                        apks: group.clone(),
                        target: target.clone(),
                        cancel,
                    })
                    .is_err()
                {
                    self.update_job(id, |j| {
                        j.stage = JobStage::Failed;
                        j.detail = "安装队列已关闭".into();
                    });
                }
            }
        }
    }

    fn update_job(&self, id: u64, action: impl FnOnce(&mut Job)) {
        if let Some(job) = self
            .state
            .lock()
            .expect("state lock poisoned")
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
        {
            action(job);
        }
    }

    async fn run_lane(self: Arc<Self>, mut receiver: mpsc::UnboundedReceiver<QueueItem>) {
        while let Some(item) = receiver.recv().await {
            if item.cancel.is_cancelled() {
                self.update_job(item.job.id, |j| j.stage = JobStage::Canceled);
            } else {
                self.update_job(item.job.id, |j| j.stage = JobStage::Connecting);
                let result = async {
                    let client = self
                        .clone()
                        .client_for_install(&item.target, &item.cancel)
                        .await
                        .map_err(|error| install::InstallError {
                            stage: if item.cancel.is_cancelled() {
                                JobStage::Canceled
                            } else {
                                JobStage::Failed
                            },
                            detail: format!("安装前连接失败，尚未传输 APK：{error:#}"),
                        })?;
                    install::install(
                        client,
                        &item.apks,
                        item.job.test_packages,
                        item.cancel,
                        |progress| {
                            self.update_job(item.job.id, |j| {
                                j.stage = progress.stage;
                                j.transferred = progress.bytes;
                                j.total = progress.total;
                                j.bytes_per_second = progress.speed;
                                if !progress.detail.is_empty() {
                                    j.detail = progress.detail;
                                }
                            });
                        },
                    )
                    .await
                }
                .await;
                match result {
                    Ok(()) => self.update_job(item.job.id, |j| {
                        j.stage = JobStage::Succeeded;
                        j.transferred = j.total;
                    }),
                    Err(error) => self.update_job(item.job.id, |j| {
                        j.stage = error.stage;
                        j.detail = error.detail;
                    }),
                }
                let detail = self
                    .state
                    .lock()
                    .expect("state lock poisoned")
                    .jobs
                    .iter()
                    .find(|j| j.id == item.job.id)
                    .map(|j| {
                        format!(
                            "安装 {} → {}：{} {}",
                            j.title,
                            j.device_name,
                            j.stage.label(),
                            j.detail
                        )
                    });
                if let Some(detail) = detail
                    && let Err(error) = self.storage.log(&detail)
                {
                    self.notice(&format!("安装记录保存失败：{error:#}"));
                }
            }
            self.cancellations
                .lock()
                .expect("cancellation lock poisoned")
                .remove(&item.job.id);
        }
    }

    async fn client_for_install(
        self: Arc<Self>,
        target: &Device,
        cancel: &CancellationToken,
    ) -> Result<Arc<AdbClient>> {
        ensure!(!cancel.is_cancelled(), "安装已取消");
        let current = self.clients.lock().await.get(&target.id).cloned();
        let client = if let Some(client) = current.filter(|c| c.state() != ConnectionState::Closed)
        {
            client
        } else if let Some(device_id) = target.id.strip_prefix("tls:") {
            self.clone()
                .connect_wireless(WirelessTarget::Paired(device_id.to_owned()), cancel)
                .await?
        } else if let Some(endpoint) = &target.endpoint {
            self.clone()
                .connect_wireless(WirelessTarget::Address(endpoint.clone()), cancel)
                .await?
        } else {
            anyhow::bail!("原目标 USB 设备已断开，请接回原设备")
        };
        if !target.serial.is_empty() {
            let state = self.state.lock().expect("state lock poisoned");
            ensure!(
                state
                    .devices
                    .iter()
                    .any(|d| d.id == target.id && d.serial == target.serial),
                "当前连接与提交时选择的设备不一致，已停止安装"
            );
        }
        ensure!(!cancel.is_cancelled(), "安装已取消");
        Ok(client)
    }
}

fn saved_devices(storage: &Storage) -> Vec<Device> {
    let settings = storage.settings();
    let mut endpoints: BTreeMap<_, _> = settings
        .endpoints
        .iter()
        .map(|e| (e.key(), Some(e.clone())))
        .collect();
    for paired in storage.paired_devices() {
        endpoints
            .entry(format!("tls:{}", paired.device_id))
            .or_insert(None);
    }
    endpoints
        .into_iter()
        .map(|(id, endpoint)| {
            let info = settings.device_info.get(&id);
            let serial = info.map(|d| d.serial.clone()).unwrap_or_default();
            let model = info
                .filter(|d| !d.model.is_empty())
                .map(|d| d.model.clone())
                .unwrap_or_else(|| {
                    endpoint
                        .as_ref()
                        .map(|e| format!("{}:{}", e.host, e.port))
                        .unwrap_or_else(|| {
                            format!("已配对设备 · {}", id.trim_start_matches("tls:"))
                        })
                });
            Device {
                name: settings
                    .device_alias(&id, &serial)
                    .map(str::to_owned)
                    .unwrap_or_else(|| model.clone()),
                model,
                serial,
                android: info.map(|d| d.android.clone()).unwrap_or_default(),
                transport: "无线".into(),
                status: DeviceStatus::Offline,
                detail: "已保存设备；选择后安装时自动尝试重连".into(),
                selected: false,
                endpoint,
                id,
            }
        })
        .collect()
}

fn parse_device_info(output: &str) -> Result<DeviceInfo> {
    let lines: Vec<_> = output.lines().map(str::trim).collect();
    ensure!(
        lines.len() == 3,
        "设备信息返回不完整或格式异常，未更新已保存的信息"
    );
    Ok(DeviceInfo {
        model: lines[0].into(),
        serial: lines[1].into(),
        android: lines[2].into(),
    })
}

pub async fn shell_text(client: &AdbClient, command: &str) -> Result<String> {
    shell_text_with_timeout(client, command, Duration::from_secs(15)).await
}

async fn read_apks(paths: Vec<PathBuf>, split: bool) -> Result<Vec<Apk>> {
    tokio::task::spawn_blocking(move || {
        let apks = paths
            .into_iter()
            .map(Apk::read)
            .collect::<Result<Vec<_>>>()?;
        validate_group(&apks, split)?;
        Ok(apks)
    })
    .await
    .context("读取 APK 任务失败")?
}

pub async fn shell_text_with_timeout(
    client: &AdbClient,
    command: &str,
    timeout: Duration,
) -> Result<String> {
    let options = if client.supports_feature("shell_v2") {
        droidmux::shell::ShellOptions::default()
    } else {
        droidmux::shell::ShellOptions::legacy()
    };
    let output = tokio::time::timeout(
        timeout,
        droidmux::shell::execute_with_options(client, command, options),
    )
    .await
    .context("设备命令超时")??;
    ensure!(
        output.exit_code.is_none_or(|c| c == 0),
        "设备命令失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.to_vec()).context("设备信息不是 UTF-8")
}
