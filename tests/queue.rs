mod support;
use droidmux::pairing::{CredentialStore, StoredPairedDevice};
use quickadb::{
    engine::Backend,
    model::{DeviceStatus, Endpoint, JobStage, PreparationState},
    storage::Storage,
};
use std::time::{Duration, Instant};
use support::{DeviceOptions, DeviceServer, TempDirectory};

fn wait_for(backend: &Backend, condition: impl Fn(&quickadb::model::Snapshot) -> bool) {
    wait_for_timeout(backend, Duration::from_secs(6), condition);
}

fn wait_for_timeout(
    backend: &Backend,
    duration: Duration,
    condition: impl Fn(&quickadb::model::Snapshot) -> bool,
) {
    let deadline = Instant::now() + duration;
    loop {
        let snapshot = backend.snapshot();
        if condition(&snapshot) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "state deadline exceeded: {}",
            snapshot.notice
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn prepared_revision(backend: &Backend) -> u64 {
    wait_for(backend, |s| {
        s.preparation
            .as_ref()
            .is_some_and(|p| matches!(p.state, PreparationState::Ready(_)))
    });
    backend
        .snapshot()
        .preparation
        .expect("prepared selection")
        .revision
}

#[test]
fn paired_devices_use_discovered_ports_and_keep_one_identity() {
    let directory = TempDirectory::new();
    let options = DeviceOptions {
        features: "shell_v2,cmd,abb_exec",
        idle_timeout: Duration::from_secs(35),
        tls: true,
        ..Default::default()
    };
    let first = DeviceServer::start(options.clone());
    let guid = format!("adb-quickadb-{}-{}", std::process::id(), first.port);
    let mdns = mdns_sd::ServiceDaemon::new().expect("advertiser");
    let publish = |port| {
        let service = mdns_sd::ServiceInfo::new(
            droidmux::discovery::ADB_TLS_CONNECT_SERVICE,
            &guid,
            &format!("{guid}.local."),
            "",
            port,
            [("name", "TLS 协议夹具")].as_slice(),
        )
        .expect("service")
        .enable_addr_auto();
        mdns.register(service).expect("register");
    };
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let runtime = tokio::runtime::Runtime::new().expect("setup runtime");
    runtime
        .block_on(storage.save_paired_device(&StoredPairedDevice {
            device_id: guid.clone(),
            host: "192.0.2.1".into(),
            certificate_fingerprint: "fixture-only".into(),
        }))
        .expect("save pairing record");
    drop(runtime);
    storage
        .update_settings(|s| {
            s.endpoints.push(Endpoint {
                host: "192.0.2.1".into(),
                port: 1,
                paired_id: Some(guid.clone()),
            })
        })
        .expect("old endpoint");
    publish(first.port);
    mdns.register(
        mdns_sd::ServiceInfo::new(
            droidmux::discovery::ADB_TLS_PAIRING_SERVICE,
            &guid,
            &format!("{guid}.local."),
            "",
            1,
            [("name", "配对端口不可用于连接")].as_slice(),
        )
        .expect("pairing advertisement")
        .enable_addr_auto(),
    )
    .expect("register pairing service");
    let backend = Backend::new(storage.clone()).expect("backend");
    backend.connect_paired(guid.clone());
    wait_for_timeout(&backend, Duration::from_secs(20), |s| {
        s.devices.iter().any(|d| d.status == DeviceStatus::Online)
    });
    std::thread::sleep(Duration::from_secs(12));
    let snapshot = backend.snapshot();
    assert_eq!(snapshot.devices.len(), 1);
    assert_eq!(
        snapshot.devices[0].status,
        DeviceStatus::Online,
        "{:?}",
        snapshot.devices
    );
    let id = snapshot.devices[0].id.clone();
    assert_eq!(
        snapshot.devices[0]
            .endpoint
            .as_ref()
            .expect("endpoint")
            .port,
        first.port
    );
    assert!(
        first
            .records
            .lock()
            .expect("records")
            .services
            .iter()
            .any(|s| s.starts_with(b"shell,v2,raw:echo quickadb"))
    );
    backend.toggle(&id);
    let apk = support::write_apk(&directory, "自动连接后.apk", "test.autopair", "", 1);
    let expected = std::fs::read(&apk).expect("APK");
    backend.submit(vec![apk.clone()], false, false);
    wait_for(&backend, |s| s.jobs.len() == 1 && !s.jobs[0].stage.active());
    assert_eq!(backend.snapshot().jobs[0].stage, JobStage::Succeeded);
    assert_eq!(
        first.records.lock().expect("records").uploads,
        vec![expected.clone()]
    );
    backend.disconnect(id.clone());
    wait_for(&backend, |s| {
        s.devices[0].status == DeviceStatus::Disconnected
    });
    let second = DeviceServer::start(options);
    publish(second.port);
    wait_for_timeout(&backend, Duration::from_secs(20), |s| {
        s.discovered
            .iter()
            .any(|d| d.paired_id() == Some(guid.as_str()) && d.endpoint.port == second.port)
    });
    assert_eq!(
        backend.snapshot().devices[0].status,
        DeviceStatus::Disconnected
    );
    backend.connect_paired(guid.clone());
    wait_for(&backend, |s| s.devices[0].status == DeviceStatus::Online);
    let snapshot = backend.snapshot();
    assert_eq!(snapshot.devices.len(), 1);
    assert_eq!(snapshot.devices[0].id, id);
    assert_eq!(
        snapshot.devices[0]
            .endpoint
            .as_ref()
            .expect("new endpoint")
            .port,
        second.port
    );
    let saved = storage.settings().endpoints;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].port, second.port);
    backend.toggle(&id);
    backend.submit(vec![apk], false, false);
    wait_for(&backend, |s| s.jobs.len() == 2 && !s.jobs[1].stage.active());
    assert_eq!(backend.snapshot().jobs[1].stage, JobStage::Succeeded);
    assert_eq!(
        second.records.lock().expect("records").uploads,
        vec![expected]
    );
    drop(backend);
    mdns.shutdown().expect("shutdown advertiser");
}

#[test]
fn real_transport_loss_marks_device_offline_and_preserves_the_reason() {
    let directory = TempDirectory::new();
    let server = DeviceServer::start(DeviceOptions {
        disconnect_after_upload: true,
        features: "shell_v2,cmd,abb_exec",
        ..Default::default()
    });
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let backend = Backend::new(storage).expect("backend");
    backend.connect(Endpoint {
        host: "127.0.0.1".into(),
        port: server.port,
        paired_id: None,
    });
    wait_for(&backend, |s| {
        s.devices.iter().any(|d| d.status == DeviceStatus::Online)
    });
    backend.toggle(&backend.snapshot().devices[0].id);
    backend.submit(
        vec![support::write_apk(
            &directory,
            "中断.apk",
            "test.loss",
            "",
            1,
        )],
        false,
        false,
    );
    wait_for(&backend, |s| {
        s.devices[0].status == DeviceStatus::Offline
            && s.jobs.len() == 1
            && !s.jobs[0].stage.active()
    });
    let snapshot = backend.snapshot();
    assert_ne!(snapshot.jobs[0].stage, JobStage::Succeeded);
    assert!(
        snapshot.devices[0]
            .detail
            .contains("transport connection closed"),
        "{}",
        snapshot.devices[0].detail
    );
    assert!(!snapshot.devices[0].detail.contains("locally"));
    assert!(!snapshot.devices[0].selected);
    drop(backend);
}

#[test]
fn selection_snapshot_and_device_queues_remain_isolated() {
    let directory = TempDirectory::new();
    let server_a = DeviceServer::start(DeviceOptions {
        delay_ack: Duration::from_millis(55),
        serial: "DEVICE-A",
        ..Default::default()
    });
    let server_b = DeviceServer::start(DeviceOptions {
        delay_ack: Duration::from_millis(55),
        serial: "DEVICE-B",
        ..Default::default()
    });
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let backend = Backend::new(storage).expect("backend");
    backend.connect(Endpoint {
        host: "127.0.0.1".into(),
        port: server_a.port,
        paired_id: None,
    });
    backend.connect(Endpoint {
        host: "127.0.0.1".into(),
        port: server_b.port,
        paired_id: None,
    });
    wait_for(&backend, |s| {
        s.devices
            .iter()
            .filter(|d| d.status == DeviceStatus::Online)
            .count()
            == 2
    });
    let devices = backend.snapshot().devices;
    assert!(devices.iter().all(|d| !d.selected));
    let first = support::write_apk(&directory, "中文 第一.apk", "test.first", "", 1);
    let second = support::write_apk(&directory, "第二.apk", "test.second", "", 2);
    backend.prepare(vec![first.clone(), second.clone()], false);
    let revision = prepared_revision(&backend);
    assert!(backend.snapshot().jobs.is_empty());
    assert!(
        server_a
            .records
            .lock()
            .expect("A records")
            .uploads
            .is_empty()
    );
    assert!(
        server_b
            .records
            .lock()
            .expect("B records")
            .uploads
            .is_empty()
    );
    backend.install_prepared(revision, false);
    assert!(backend.snapshot().preparation.is_some());
    assert!(backend.snapshot().jobs.is_empty());
    for device in &devices {
        backend.toggle(&device.id);
    }
    backend.install_prepared(revision, false);
    backend.install_prepared(revision, false);
    assert!(backend.snapshot().preparation.is_none());
    backend.select_all(false);
    wait_for(&backend, |s| {
        s.jobs
            .iter()
            .filter(|j| j.stage == JobStage::Transferring)
            .count()
            == 2
    });
    wait_for(&backend, |s| {
        s.jobs.len() == 4 && s.jobs.iter().all(|j| !j.stage.active())
    });
    let snapshot = backend.snapshot();
    assert!(
        snapshot.jobs.iter().all(|j| j.stage == JobStage::Succeeded),
        "{:?}",
        snapshot.jobs
    );
    assert!(snapshot.devices.iter().all(|d| !d.selected));
    let a = server_a.records.lock().expect("A records");
    let b = server_b.records.lock().expect("B records");
    let expected = vec![
        std::fs::read(first).expect("first"),
        std::fs::read(second).expect("second"),
    ];
    assert_eq!(a.uploads, expected);
    assert_eq!(b.uploads, expected);
    drop(a);
    drop(b);
    backend.submit(
        vec![support::write_apk(
            &directory,
            "不应安装.apk",
            "test.unselected",
            "",
            3,
        )],
        false,
        false,
    );
    assert_eq!(backend.snapshot().jobs.len(), 4);
    assert!(backend.snapshot().notice.contains("请先点击选择"));
    drop(backend);
}

#[test]
fn prepared_files_are_not_silently_replaced_before_installing() {
    let directory = TempDirectory::new();
    let server = DeviceServer::start(DeviceOptions::default());
    let backend =
        Backend::new(Storage::open(directory.0.join("data")).expect("storage")).expect("backend");
    backend.connect(Endpoint {
        host: "127.0.0.1".into(),
        port: server.port,
        paired_id: None,
    });
    wait_for(&backend, |s| {
        s.devices.iter().any(|d| d.status == DeviceStatus::Online)
    });
    let apk = support::write_apk(&directory, "被替换.apk", "test.original", "", 1);
    backend.prepare(vec![apk.clone()], false);
    let revision = prepared_revision(&backend);
    std::fs::write(&apk, b"changed after preparation").expect("replace file");
    backend.select_all(true);
    backend.install_prepared(revision, false);
    wait_for(&backend, |s| s.jobs.len() == 1 && !s.jobs[0].stage.active());
    assert_eq!(backend.snapshot().jobs[0].stage, JobStage::Failed);
    assert!(backend.snapshot().jobs[0].detail.contains("发生变化"));
    assert!(server.records.lock().expect("records").uploads.is_empty());
    drop(backend);
}

#[test]
fn split_preparation_requires_valid_group_and_explicit_install() {
    let directory = TempDirectory::new();
    let server = DeviceServer::start(DeviceOptions::default());
    let backend =
        Backend::new(Storage::open(directory.0.join("data")).expect("storage")).expect("backend");
    backend.connect(Endpoint {
        host: "127.0.0.1".into(),
        port: server.port,
        paired_id: None,
    });
    wait_for(&backend, |s| {
        s.devices.iter().any(|d| d.status == DeviceStatus::Online)
    });
    backend.select_all(true);
    let base = support::write_apk(&directory, "base.apk", "test.split", "", 7);
    let split = support::write_apk(&directory, "config.zh.apk", "test.split", "config.zh", 7);
    backend.prepare(vec![base.clone(), split.clone()], false);
    wait_for(&backend, |s| {
        s.preparation
            .as_ref()
            .is_some_and(|p| matches!(p.state, PreparationState::Failed(_)))
    });
    let invalid = backend.snapshot().preparation.expect("failed selection");
    backend.install_prepared(invalid.revision, false);
    assert!(backend.snapshot().jobs.is_empty());
    backend.prepare(vec![base.clone(), split.clone()], true);
    let revision = prepared_revision(&backend);
    backend.install_prepared(invalid.revision, false);
    assert!(backend.snapshot().preparation.is_some());
    assert!(backend.snapshot().jobs.is_empty());
    assert!(server.records.lock().expect("records").uploads.is_empty());
    backend.install_prepared(revision, false);
    wait_for(&backend, |s| s.jobs.len() == 1 && !s.jobs[0].stage.active());
    assert_eq!(backend.snapshot().jobs[0].stage, JobStage::Succeeded);
    assert_eq!(
        server.records.lock().expect("records").uploads,
        vec![
            std::fs::read(base).expect("base"),
            std::fs::read(split).expect("split")
        ]
    );
    backend.prepare(vec![directory.0.join("missing.apk")], false);
    backend.clear_preparation();
    backend.prepare(vec![directory.0.join("base.apk")], false);
    let new_revision = prepared_revision(&backend);
    assert!(new_revision > revision);
    backend.clear_preparation();
    assert!(backend.snapshot().preparation.is_none());
    assert_eq!(backend.snapshot().jobs.len(), 1);
    drop(backend);
}

#[test]
fn rejected_status_command_does_not_close_a_usable_connection() {
    let directory = TempDirectory::new();
    let server = DeviceServer::start(DeviceOptions {
        features: "shell_v2,cmd,abb_exec",
        reject_heartbeat: true,
        idle_timeout: Duration::from_secs(30),
        ..Default::default()
    });
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let backend = Backend::new(storage).expect("backend");
    backend.connect(Endpoint {
        host: "127.0.0.1".into(),
        port: server.port,
        paired_id: None,
    });
    wait_for(&backend, |s| {
        s.devices.iter().any(|d| d.status == DeviceStatus::Online)
    });
    std::thread::sleep(Duration::from_secs(12));
    let snapshot = backend.snapshot();
    assert_eq!(
        snapshot.devices[0].status,
        DeviceStatus::Online,
        "{:?}",
        snapshot.devices
    );
    assert!(snapshot.devices[0].detail.contains("状态检查失败"));
    assert!(
        server
            .records
            .lock()
            .expect("records")
            .services
            .iter()
            .any(|s| s.ends_with(b"echo quickadb\0"))
    );
    backend.toggle(&snapshot.devices[0].id);
    let apk = support::write_apk(&directory, "状态检查后.apk", "test.connected", "", 1);
    let bytes = std::fs::read(&apk).expect("APK bytes");
    backend.submit(vec![apk], false, false);
    wait_for(&backend, |s| s.jobs.len() == 1 && !s.jobs[0].stage.active());
    assert_eq!(backend.snapshot().jobs[0].stage, JobStage::Succeeded);
    assert_eq!(server.records.lock().expect("records").uploads, vec![bytes]);
    drop(backend);
}

#[test]
fn persisted_key_is_protected_and_corrupt_settings_are_reported() {
    let directory = TempDirectory::new();
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let encoded = std::fs::read(storage.directory.join("host.key")).expect("protected key");
    assert!(!String::from_utf8_lossy(&encoded).contains("PRIVATE KEY"));
    let first = storage
        .credential()
        .expect("credential")
        .to_pkcs8_pem()
        .expect("PEM");
    storage
        .update_settings(|s| s.pinned = true)
        .expect("preferences");
    let directory = storage.directory.clone();
    drop(storage);
    let reopened = Storage::open(directory.clone()).expect("reopened storage");
    assert!(reopened.settings().pinned);
    assert_eq!(
        reopened
            .credential()
            .expect("credential")
            .to_pkcs8_pem()
            .expect("PEM")
            .as_str(),
        first.as_str()
    );
    drop(reopened);
    std::fs::write(directory.join("settings.json"), b"invalid json").expect("corrupt fixture");
    assert!(Storage::open(directory).is_err());
}
