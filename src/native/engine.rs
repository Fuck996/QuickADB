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
    paths: Vec<PathBuf>,
    split: bool,
    test: bool,
    targets: Vec<Device>,
}

struct QueueItem {
    job: Job,
    apks: Vec<Apk>,
    client: Arc<AdbClient>,
    cancel: CancellationToken,
}

struct Engine {
    state: Mutex<Snapshot>,
    storage: Arc<Storage>,
    credential: Arc<RsaAdbCredential>,
    clients: AsyncMutex<BTreeMap<String, Arc<AdbClient>>>,
    usb: Mutex<BTreeMap<String, UsbDeviceInfo>>,
    connecting: Mutex<BTreeSet<String>>,
    lanes: AsyncMutex<BTreeMap<String, mpsc::UnboundedSender<QueueItem>>>,
    cancellations: Mutex<BTreeMap<u64, CancellationToken>>,
    next_job: AtomicU64,
    stop: CancellationToken,
}

impl Backend {
    pub fn new(storage: Arc<Storage>) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(3)
            .build()?;
        let engine = Arc::new(Engine {
            credential: storage.credential()?,
            storage,
            state: Mutex::new(Snapshot::default()),
            clients: AsyncMutex::new(BTreeMap::new()),
            usb: Mutex::new(BTreeMap::new()),
            connecting: Mutex::new(BTreeSet::new()),
            lanes: AsyncMutex::new(BTreeMap::new()),
            cancellations: Mutex::new(BTreeMap::new()),
            next_job: AtomicU64::new(1),
            stop: CancellationToken::new(),
        });
        runtime.spawn(engine.clone().monitor());
        let (submissions, mut pending) = mpsc::unbounded_channel::<Submission>();
        let submit_engine = engine.clone();
        runtime.spawn(async move {
            while let Some(request) = pending.recv().await {
                submit_engine
                    .clone()
                    .submit(request.paths, request.split, request.test, request.targets)
                    .await;
            }
        });
        for endpoint in engine.storage.settings().endpoints {
            runtime.spawn(engine.clone().connect_endpoint(endpoint));
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
        self.engine
            .state
            .lock()
            .expect("state lock poisoned")
            .clone()
    }

    pub fn toggle(&self, id: &str) {
        if let Some(device) = self
            .engine
            .state
            .lock()
            .expect("state lock poisoned")
            .devices
            .iter_mut()
            .find(|d| d.id == id && d.status == DeviceStatus::Online)
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
            if device.status == DeviceStatus::Online {
                device.selected = selected;
            }
        }
    }

    pub fn connect(&self, endpoint: Endpoint) {
        self.spawn(self.engine.clone().connect_endpoint(endpoint));
    }

    pub fn pair(&self, host: String, pairing_port: u16, code: String, connection_port: u16) {
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
                    engine.notice("配对成功，正在连接无线调试端口");
                    engine
                        .connect_endpoint(Endpoint {
                            host,
                            port: connection_port,
                            paired_id: Some(result.device_id),
                        })
                        .await;
                }
                Err(error) => engine.notice(&format!("无线配对失败：{error}")),
            }
        });
    }

    pub fn retry_connection(&self, id: String) {
        let endpoint = self
            .snapshot()
            .devices
            .iter()
            .find(|d| d.id == id)
            .and_then(|d| d.endpoint.clone());
        if let Some(endpoint) = endpoint {
            self.connect(endpoint);
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

    pub fn submit(&self, paths: Vec<PathBuf>, split: bool, test: bool) {
        let targets: Vec<_> = self
            .snapshot()
            .devices
            .into_iter()
            .filter(|d| d.selected && d.status == DeviceStatus::Online)
            .collect();
        if targets.is_empty() {
            self.engine
                .notice("请先点击选择至少一台已连接设备，再添加 APK");
            return;
        }
        if self
            .submissions
            .send(Submission {
                paths,
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
            .find(|d| d.id == job.device_id && d.status == DeviceStatus::Online)
        else {
            self.engine.notice("原目标设备未连接，请连接原设备后重试");
            return;
        };
        if self
            .submissions
            .send(Submission {
                paths: job.paths.clone(),
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
            .notice
            .clear();
    }

    pub fn notify(&self, message: &str) {
        self.engine.notice(message);
    }

    pub fn set_alias(&self, device: &Device, alias: &str) {
        match self.engine.storage.update_settings(|settings| {
            if alias.trim().is_empty() {
                settings.aliases.remove(&device.serial);
            } else {
                settings
                    .aliases
                    .insert(device.serial.clone(), alias.trim().into());
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
        self.state.lock().expect("state lock poisoned").notice = match log {
            Ok(()) => message.to_owned(),
            Err(error) => format!("{message}\n日志保存失败：{error:#}"),
        };
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
        self.finish_connect(id, result).await;
    }

    async fn connect_endpoint(self: Arc<Self>, endpoint: Endpoint) {
        let id = endpoint.key();
        if self.clients.lock().await.contains_key(&id) {
            self.notice("此设备连接已经打开");
            return;
        }
        if !self.start_connect(
            &id,
            &format!("{}:{}", endpoint.host, endpoint.port),
            Some(endpoint.clone()),
        ) {
            return;
        }
        let result = async {
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
            let observer_id = id.clone();
            let client = tokio::time::timeout(
                Duration::from_secs(120),
                AdbClient::connect_with_observer(
                    transport,
                    self.credential.clone(),
                    move |state| {
                        if state == ConnectionState::AwaitingAuthorization {
                            engine.update_device(&observer_id, |d| {
                                d.status = DeviceStatus::Unauthorized
                            });
                        }
                    },
                ),
            )
            .await
            .context("等待无线设备授权超时")??;
            self.storage.update_settings(|settings| {
                if !settings.endpoints.iter().any(|e| e.key() == endpoint.key()) {
                    settings.endpoints.push(endpoint.clone());
                }
            })?;
            Ok::<_, anyhow::Error>(client)
        }
        .await;
        self.finish_connect(id, result).await;
    }

    async fn finish_connect(self: &Arc<Self>, id: String, result: Result<AdbClient>) {
        match result {
            Ok(client) => {
                let info = shell_text(&client, "getprop ro.product.model; getprop ro.serialno; getprop ro.build.version.release").await;
                match info {
                    Ok(output) => {
                        let lines: Vec<_> = output.lines().collect();
                        let model = lines.first().copied().unwrap_or("").trim();
                        let serial = lines.get(1).copied().unwrap_or("").trim();
                        let android = lines.get(2).copied().unwrap_or("").trim();
                        let alias = self.storage.settings().aliases.get(serial).cloned();
                        self.update_device(&id, |d| {
                            if !model.is_empty() {
                                d.model = model.into();
                                d.name = alias.unwrap_or_else(|| model.into());
                            }
                            d.serial = serial.into();
                            d.android = android.into();
                            d.status = DeviceStatus::Online;
                            d.detail.clear();
                        });
                        let client = Arc::new(client);
                        self.clients.lock().await.insert(id.clone(), client.clone());
                        tokio::spawn(self.clone().watch_connection(id.clone(), client));
                    }
                    Err(error) => {
                        self.update_device(&id, |d| {
                            d.status = DeviceStatus::Offline;
                            d.detail = format!("读取设备信息失败：{error:#}");
                            d.selected = false;
                        });
                        if let Err(error) = client.close().await {
                            self.notice(&format!("读取设备信息失败后关闭连接失败：{error}"));
                        }
                    }
                }
            }
            Err(error) => self.update_device(&id, |d| {
                d.status = DeviceStatus::Offline;
                d.detail = format!("{error:#}");
                d.selected = false;
            }),
        }
        self.connecting
            .lock()
            .expect("connecting lock poisoned")
            .remove(&id);
    }

    async fn watch_connection(self: Arc<Self>, id: String, client: Arc<AdbClient>) {
        loop {
            tokio::select! { _ = self.stop.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(10)) => {} }
            let current = self
                .clients
                .lock()
                .await
                .get(&id)
                .is_some_and(|c| Arc::ptr_eq(c, &client));
            if !current {
                break;
            }
            let result = shell_text(&client, "echo quickadb").await;
            if let Err(error) = result {
                self.clients.lock().await.remove(&id);
                self.update_device(&id, |d| {
                    d.status = DeviceStatus::Offline;
                    d.selected = false;
                    d.detail = format!("连接中断：{error:#}");
                });
                if let Err(error) = client.close().await {
                    self.notice(&format!("关闭中断连接：{error}"));
                }
                break;
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
                while let Ok(device) = receiver.try_recv() {
                    let addresses = device.ipv4_addresses();
                    if let Some(address) = addresses.first() {
                        let endpoint = Endpoint {
                            host: address.to_string(),
                            port: device.port,
                            paired_id: None,
                        };
                        let mut state = self.state.lock().expect("state lock poisoned");
                        if !state
                            .discovered
                            .iter()
                            .any(|e| e.host == endpoint.host && e.port == endpoint.port)
                        {
                            state.discovered.push(endpoint);
                        }
                    }
                }
            }
            tokio::select! { _ = self.stop.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(2)) => {} }
        }
    }

    async fn submit(
        self: Arc<Self>,
        paths: Vec<PathBuf>,
        split: bool,
        test: bool,
        targets: Vec<Device>,
    ) {
        let apks = tokio::task::spawn_blocking(move || {
            paths.into_iter().map(Apk::read).collect::<Result<Vec<_>>>()
        })
        .await;
        let apks = match apks {
            Ok(Ok(apks)) => apks,
            Ok(Err(error)) => {
                self.notice(&format!("无法添加 APK：{error:#}"));
                return;
            }
            Err(error) => {
                self.notice(&format!("读取 APK 任务失败：{error}"));
                return;
            }
        };
        if let Err(error) = validate_group(&apks, split) {
            self.notice(&format!("无法添加 APK：{error:#}"));
            return;
        }
        let groups = if split {
            vec![apks]
        } else {
            apks.into_iter().map(|p| vec![p]).collect()
        };
        let clients = self.clients.lock().await;
        for target in targets {
            let client = clients.get(&target.id).cloned();
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
                if client.is_none() {
                    let mut job = job;
                    job.stage = JobStage::Failed;
                    job.detail = "原目标设备在 APK 检查期间断开，请连接原设备后重试".into();
                    self.state
                        .lock()
                        .expect("state lock poisoned")
                        .jobs
                        .push(job);
                    continue;
                }
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
                        client: client.as_ref().expect("checked client").clone(),
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
                let result = install::install(
                    item.client,
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
}

pub async fn shell_text(client: &AdbClient, command: &str) -> Result<String> {
    shell_text_with_timeout(client, command, Duration::from_secs(15)).await
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
