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
    let second = DeviceServer::start(options.clone());
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
    backend.toggle(&id);
    backend.prepare(vec![apk.clone()], false);
    backend.install_prepared(prepared_revision(&backend), false);
    backend.retry_connection(id.clone());
    wait_for(&backend, |s| s.jobs.len() == 2 && !s.jobs[1].stage.active());
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
    assert_eq!(backend.snapshot().jobs[1].stage, JobStage::Succeeded);
    assert_eq!(
        second.records.lock().expect("records").uploads,
        vec![expected.clone()]
    );
    backend.set_alias(&snapshot.devices[0], "常用测试机");
    let data = storage.directory.clone();
    drop(backend);
    drop(storage);
    mdns.unregister(&format!(
        "{guid}.{}",
        droidmux::discovery::ADB_TLS_CONNECT_SERVICE
    ))
    .expect("withdraw");
    let restored =
        Backend::new(Storage::open(data).expect("reopen data")).expect("restarted backend");
    let snapshot = restored.snapshot();
    assert_eq!(snapshot.devices.len(), 1);
    assert_eq!(snapshot.devices[0].id, id);
    assert_eq!(snapshot.devices[0].name, "常用测试机");
    assert_eq!(snapshot.devices[0].serial, "QUICKADB-TEST");
    assert_eq!(snapshot.devices[0].android, "14");
    assert!(snapshot.devices[0].selectable());
    assert!(!snapshot.devices[0].selected);
    restored.toggle(&id);
    restored.submit(vec![apk], false, false);
    let third = DeviceServer::start(options);
    publish(third.port);
    wait_for_timeout(&restored, Duration::from_secs(20), |s| {
        s.jobs.len() == 1 && !s.jobs[0].stage.active()
    });
    let snapshot = restored.snapshot();
    assert_eq!(
        snapshot.jobs[0].stage,
        JobStage::Succeeded,
        "{}",
        snapshot.jobs[0].detail
    );
    assert_eq!(snapshot.devices.len(), 1);
    assert_eq!(snapshot.devices[0].name, "常用测试机");
    assert_eq!(
        snapshot.devices[0]
            .endpoint
            .as_ref()
            .expect("current address")
            .port,
        third.port
    );
    assert_eq!(
        third.records.lock().expect("records").uploads,
        vec![expected]
    );
    drop(restored);
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
    assert!(snapshot.devices[0].selected);
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

#[test]
fn offline_install_waits_for_its_guid_and_recovers_after_cancel_and_timeout() {
    let directory = TempDirectory::new();
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let guid = format!(
        "adb-wait-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime
        .block_on(storage.save_paired_device(&StoredPairedDevice {
            device_id: guid.clone(),
            host: "192.0.2.1".into(),
            certificate_fingerprint: "fixture-only".into(),
        }))
        .expect("paired record without an old endpoint");
    drop(runtime);
    let backend = Backend::new(storage.clone()).expect("backend");
    let id = format!("tls:{guid}");
    assert_eq!(backend.snapshot().devices[0].id, id);
    assert_eq!(backend.snapshot().devices[0].status, DeviceStatus::Offline);
    assert!(backend.snapshot().devices[0].endpoint.is_none());
    backend.toggle(&id);
    let apk = support::write_apk(&directory, "等待无线.apk", "test.wait", "", 1);
    backend.submit(vec![apk.clone(), apk.clone()], false, false);
    backend.select_all(false);
    wait_for(&backend, |s| {
        s.jobs.len() == 2 && s.jobs[0].stage == JobStage::Connecting
    });
    let first = backend.snapshot().jobs[0].id;
    backend.cancel(first);
    wait_for(&backend, |s| {
        s.jobs[0].stage == JobStage::Canceled && s.jobs[1].stage == JobStage::Connecting
    });
    wait_for_timeout(&backend, Duration::from_secs(20), |s| {
        s.jobs[1].stage == JobStage::Failed
    });
    let snapshot = backend.snapshot();
    assert!(snapshot.jobs[1].detail.contains("尚未发现"));
    assert_eq!(snapshot.jobs[1].transferred, 0);
    assert_eq!(storage.paired_devices().len(), 1);

    let server = DeviceServer::start(DeviceOptions {
        tls: true,
        ..Default::default()
    });
    let mdns = mdns_sd::ServiceDaemon::new().expect("advertiser");
    let publish = |instance: &str| {
        mdns.register(
            mdns_sd::ServiceInfo::new(
                droidmux::discovery::ADB_TLS_CONNECT_SERVICE,
                instance,
                &format!("{instance}.local."),
                "",
                server.port,
                [("name", "同地址不同身份")].as_slice(),
            )
            .expect("service")
            .enable_addr_auto(),
        )
        .expect("register");
    };
    let other_guid = format!("other-{guid}");
    publish(&other_guid);
    wait_for_timeout(&backend, Duration::from_secs(12), |s| {
        s.discovered
            .iter()
            .any(|d| d.paired_id() == Some(other_guid.as_str()))
    });
    backend.retry_job(snapshot.jobs[1].id);
    wait_for(&backend, |s| {
        s.jobs.len() == 3 && s.jobs[2].stage == JobStage::Connecting
    });
    assert!(server.records.lock().expect("records").services.is_empty());
    publish(&guid);
    wait_for_timeout(&backend, Duration::from_secs(20), |s| {
        s.jobs[2].stage == JobStage::Succeeded
    });
    let snapshot = backend.snapshot();
    assert_eq!(snapshot.devices.len(), 1);
    assert!(!snapshot.devices[0].selected);
    assert_eq!(snapshot.jobs[2].device_id, id);
    assert_eq!(
        server.records.lock().expect("records").uploads,
        vec![std::fs::read(apk).expect("bytes")]
    );
    drop(backend);
    mdns.shutdown().expect("shutdown advertiser");
}

#[test]
fn remembered_tcp_device_rejects_an_address_reused_by_another_phone() {
    let directory = TempDirectory::new();
    let server = DeviceServer::start(DeviceOptions {
        serial: "OTHER-PHONE",
        ..Default::default()
    });
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port: server.port,
        paired_id: None,
    };
    storage
        .update_settings(|settings| {
            settings.device_info.insert(
                endpoint.key(),
                quickadb::model::DeviceInfo {
                    model: "原设备".into(),
                    serial: "ORIGINAL-PHONE".into(),
                    android: "14".into(),
                },
            );
            settings.endpoints.push(endpoint);
        })
        .expect("saved device");
    let backend = Backend::new(storage.clone()).expect("backend");
    wait_for(&backend, |s| {
        s.devices[0].status == DeviceStatus::Offline && s.devices[0].detail.contains("序列号")
    });
    assert_eq!(backend.snapshot().devices[0].serial, "ORIGINAL-PHONE");
    assert_eq!(
        storage
            .settings()
            .device_info
            .values()
            .next()
            .expect("original info")
            .serial,
        "ORIGINAL-PHONE"
    );
    assert!(server.records.lock().expect("records").uploads.is_empty());
    drop(backend);
}

#[test]
fn missing_properties_keep_saved_model_and_custom_name_after_restart() {
    let directory = TempDirectory::new();
    let server = DeviceServer::start(DeviceOptions {
        features: "shell_v2,cmd,abb_exec",
        device_info: Some("\nQUICKADB-TEST\n\n"),
        ..Default::default()
    });
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port: server.port,
        paired_id: None,
    };
    let id = endpoint.key();
    let data = directory.0.join("data");
    let storage = Storage::open(data.clone()).expect("storage");
    let saved = quickadb::model::DeviceInfo {
        model: "已读取的手机型号".into(),
        serial: "QUICKADB-TEST".into(),
        android: "16".into(),
    };
    storage
        .update_settings(|settings| {
            settings.endpoints.push(endpoint);
            settings.device_info.insert(id.clone(), saved.clone());
            settings
                .aliases
                .insert(saved.serial.clone(), "人工命名的测试机".into());
        })
        .expect("saved name");
    let backend = Backend::new(storage.clone()).expect("backend");
    wait_for(&backend, |s| s.devices[0].status == DeviceStatus::Online);
    let device = backend.snapshot().devices.remove(0);
    assert_eq!(device.name, "人工命名的测试机");
    assert_eq!(device.model, saved.model);
    assert_eq!(device.android, saved.android);
    assert!(device.detail.contains("未返回"));
    assert_eq!(storage.settings().device_info[&id], saved);
    assert_eq!(storage.settings().aliases[&id], "人工命名的测试机");
    drop(backend);
    drop(storage);

    let storage = Storage::open(data).expect("reopen disk storage");
    assert_eq!(storage.settings().device_info[&id], saved);
    let backend = Backend::new(storage).expect("restarted backend");
    let device = backend.snapshot().devices.remove(0);
    assert_eq!(device.name, "人工命名的测试机");
    assert_eq!(device.model, saved.model);
    assert_eq!(device.android, "16");
    assert!(!device.selected);
    drop(backend);
}

#[test]
fn incomplete_property_response_does_not_replace_saved_identity_or_name() {
    let directory = TempDirectory::new();
    let server = DeviceServer::start(DeviceOptions {
        device_info: Some("truncated response\n"),
        ..Default::default()
    });
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port: server.port,
        paired_id: None,
    };
    let id = endpoint.key();
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let saved = quickadb::model::DeviceInfo {
        model: "原设备型号".into(),
        serial: "QUICKADB-TEST".into(),
        android: "14".into(),
    };
    storage
        .update_settings(|settings| {
            settings.endpoints.push(endpoint);
            settings.device_info.insert(id.clone(), saved.clone());
            settings.aliases.insert(id.clone(), "我的测试手机".into());
        })
        .expect("saved info");
    let backend = Backend::new(storage.clone()).expect("backend");
    wait_for(&backend, |s| {
        s.devices[0].status == DeviceStatus::Offline && s.devices[0].detail.contains("返回不完整")
    });
    let device = backend.snapshot().devices.remove(0);
    assert_eq!(device.name, "我的测试手机");
    assert_eq!(storage.settings().device_info[&id], saved);
    assert!(server.records.lock().expect("records").uploads.is_empty());
    drop(backend);
}

#[test]
fn old_address_only_settings_recover_legacy_custom_name_on_verified_connection() {
    let directory = TempDirectory::new();
    let server = DeviceServer::start(DeviceOptions::default());
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port: server.port,
        paired_id: None,
    };
    let id = endpoint.key();
    let data = directory.0.join("data");
    std::fs::create_dir_all(&data).expect("data directory");
    std::fs::write(
        data.join("settings.json"),
        serde_json::to_vec(&serde_json::json!({
            "endpoints": [endpoint],
            "aliases": {"QUICKADB-TEST": "用户原来的备注"}
        }))
        .expect("old settings"),
    )
    .expect("write old settings");
    let storage = Storage::open(data.clone()).expect("storage");
    assert!(storage.settings().device_info.is_empty());
    let backend = Backend::new(storage.clone()).expect("backend");
    wait_for(&backend, |s| s.devices[0].status == DeviceStatus::Online);
    assert_eq!(backend.snapshot().devices[0].name, "用户原来的备注");
    assert_eq!(storage.settings().aliases[&id], "用户原来的备注");
    assert_eq!(
        storage.settings().device_info[&id].model,
        "Protocol Test Device"
    );
    let device = backend.snapshot().devices.remove(0);
    backend.set_alias(&device, "新的设备名字");
    assert_eq!(backend.snapshot().devices[0].name, "新的设备名字");
    drop(backend);
    drop(storage);

    let backend = Backend::new(Storage::open(data.clone()).expect("reopen")).expect("restart");
    let device = backend.snapshot().devices.remove(0);
    assert_eq!(device.name, "新的设备名字");
    backend.set_alias(&device, "");
    assert_eq!(backend.snapshot().devices[0].name, "Protocol Test Device");
    drop(backend);
    let backend = Backend::new(Storage::open(data).expect("reopen")).expect("restart");
    assert_eq!(backend.snapshot().devices[0].name, "Protocol Test Device");
    drop(backend);
}

#[test]
fn offline_paired_device_keeps_custom_name_without_a_serial_number() {
    let directory = TempDirectory::new();
    let data = directory.0.join("data");
    let storage = Storage::open(data.clone()).expect("storage");
    let runtime = tokio::runtime::Runtime::new().expect("pairing runtime");
    runtime
        .block_on(storage.save_paired_device(&StoredPairedDevice {
            device_id: "offline-named-device".into(),
            host: "192.0.2.20".into(),
            certificate_fingerprint: "fixture-only".into(),
        }))
        .expect("save pairing record");
    drop(runtime);
    let backend = Backend::new(storage.clone()).expect("backend");
    let device = backend.snapshot().devices.remove(0);
    assert!(device.serial.is_empty());
    backend.set_alias(&device, "离线也要记住的名字");
    assert_eq!(backend.snapshot().devices[0].name, "离线也要记住的名字");
    drop(backend);
    drop(storage);
    let backend = Backend::new(Storage::open(data).expect("reopen")).expect("restart");
    let device = backend.snapshot().devices.remove(0);
    assert_eq!(device.name, "离线也要记住的名字");
    assert_eq!(device.id, "tls:offline-named-device");
    assert_eq!(device.status, DeviceStatus::Offline);
    assert!(!device.selected);
    drop(backend);
}
