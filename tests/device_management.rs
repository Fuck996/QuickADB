// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

mod support;

use droidmux::pairing::{CredentialStore, StoredPairedDevice};
use quickadb::{
    engine::Backend,
    model::{DeviceInfo, DeviceStatus, Endpoint, JobStage, Snapshot},
    storage::Storage,
};
use std::time::{Duration, Instant};
use support::{DeviceOptions, DeviceServer, TempDirectory};

fn wait_for(backend: &Backend, condition: impl Fn(&Snapshot) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
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

fn address(port: u16) -> Endpoint {
    Endpoint {
        host: "127.0.0.1".into(),
        port,
        paired_id: None,
    }
}

fn pair(storage: &Storage, id: &str) {
    let runtime = tokio::runtime::Runtime::new().expect("pairing runtime");
    runtime
        .block_on(storage.save_paired_device(&StoredPairedDevice {
            device_id: id.into(),
            host: "192.0.2.1".into(),
            certificate_fingerprint: "fixture-only".into(),
        }))
        .expect("pairing record");
}

#[test]
fn failed_connection_can_be_edited_and_forgotten_without_leaving_old_rows() {
    let directory = TempDirectory::new();
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let key = std::fs::read(storage.directory.join("host.key")).expect("protected host key");
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").expect("unused address");
    let old = address(reserved.local_addr().expect("address").port());
    let server = DeviceServer::start(DeviceOptions {
        idle_timeout: Duration::from_secs(25),
        ..Default::default()
    });
    drop(reserved);
    let backend = Backend::new(storage.clone()).expect("backend");
    backend.connect(old.clone());
    wait_for(&backend, |s| {
        s.devices
            .iter()
            .any(|d| d.id == old.key() && d.status == DeviceStatus::Offline)
    });
    let original = backend
        .snapshot()
        .devices
        .into_iter()
        .find(|d| d.id == old.key())
        .expect("failed device");
    backend.set_alias(&original, "备用测试机");
    backend.toggle(&old.key());
    let updated = address(server.port);
    backend.update_connection(old.key(), Some(updated.clone()));
    wait_for(&backend, |s| {
        s.devices
            .iter()
            .any(|d| d.id == updated.key() && d.status == DeviceStatus::Online)
    });
    assert!(!backend.snapshot().devices.iter().any(|d| d.id == old.key()));
    let device = backend
        .snapshot()
        .devices
        .into_iter()
        .find(|d| d.id == updated.key())
        .expect("updated device");
    assert_eq!(device.name, "备用测试机");
    assert!(device.selected);
    assert_eq!(storage.settings().endpoints, vec![updated.clone()]);
    assert!(!storage.settings().aliases.contains_key(&old.key()));
    backend.forget_device(updated.key());
    wait_for(&backend, |s| {
        !s.managing.contains(&updated.key()) && !s.devices.iter().any(|d| d.id == updated.key())
    });
    assert!(storage.settings().endpoints.is_empty());
    assert!(!storage.settings().device_info.contains_key(&updated.key()));
    assert!(!storage.settings().aliases.contains_key(&updated.key()));
    assert_eq!(
        std::fs::read(storage.directory.join("host.key")).expect("key retained"),
        key
    );
    drop(backend);
    let reopened = Storage::open(storage.directory.clone()).expect("reopened storage");
    assert!(reopened.settings().endpoints.is_empty());
}

#[test]
fn forgetting_a_paired_device_cancels_discovery_and_preserves_other_authorizations() {
    let directory = TempDirectory::new();
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let id = format!("adb-forget-{}", std::process::id());
    let other = format!("adb-keep-{}", std::process::id());
    pair(&storage, &id);
    pair(&storage, &other);
    let key = std::fs::read(storage.directory.join("host.key")).expect("host key");
    storage
        .update_settings(|s| {
            s.aliases.insert(format!("tls:{id}"), "待删除设备".into());
            s.aliases.insert(format!("tls:{other}"), "保留设备".into());
        })
        .expect("aliases");
    let backend = Backend::new(storage.clone()).expect("backend");
    backend.connect_paired(id.clone());
    let device_id = format!("tls:{id}");
    wait_for(&backend, |s| {
        s.devices
            .iter()
            .any(|d| d.id == device_id && d.status == DeviceStatus::Connecting)
    });
    backend.forget_device(device_id.clone());
    wait_for(&backend, |s| {
        !s.managing.contains(&device_id) && !s.devices.iter().any(|d| d.id == device_id)
    });
    let mdns = mdns_sd::ServiceDaemon::new().expect("advertiser");
    mdns.register(
        mdns_sd::ServiceInfo::new(
            droidmux::discovery::ADB_TLS_CONNECT_SERVICE,
            &id,
            &format!("{id}.local."),
            "",
            12345,
            [("name", "已忘记的无线设备")].as_slice(),
        )
        .expect("service")
        .enable_addr_auto(),
    )
    .expect("register");
    wait_for(&backend, |s| {
        s.discovered
            .iter()
            .any(|d| d.paired_id() == Some(id.as_str()))
    });
    std::thread::sleep(Duration::from_millis(250));
    assert!(!backend.snapshot().devices.iter().any(|d| d.id == device_id));
    assert!(!backend.snapshot().notice.contains("失败"));
    assert_eq!(storage.paired_devices().len(), 1);
    assert_eq!(storage.paired_devices()[0].device_id, other);
    assert!(
        storage
            .settings()
            .aliases
            .contains_key(&format!("tls:{other}"))
    );
    assert_eq!(
        std::fs::read(storage.directory.join("host.key")).expect("host key"),
        key
    );
    mdns.shutdown().expect("shutdown advertiser");
    drop(backend);
    let reopened = Storage::open(storage.directory.clone()).expect("reopen");
    assert!(reopened.paired_devices().iter().all(|d| d.device_id != id));
    let restarted = Backend::new(reopened).expect("restart");
    assert!(
        !restarted
            .snapshot()
            .devices
            .iter()
            .any(|d| d.id == device_id)
    );
    assert!(
        restarted
            .snapshot()
            .devices
            .iter()
            .any(|d| d.name == "保留设备")
    );
}

#[test]
fn changing_or_forgetting_an_install_target_does_not_interrupt_or_retarget_its_job() {
    let directory = TempDirectory::new();
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let server = DeviceServer::start(DeviceOptions {
        delay_ack: Duration::from_millis(25),
        idle_timeout: Duration::from_secs(25),
        ..Default::default()
    });
    let endpoint = address(server.port);
    let backend = Backend::new(storage.clone()).expect("backend");
    backend.connect(endpoint.clone());
    wait_for(&backend, |s| {
        s.devices
            .iter()
            .any(|d| d.id == endpoint.key() && d.status == DeviceStatus::Online)
    });
    backend.toggle(&endpoint.key());
    let apk = support::write_apk(&directory, "install.apk", "test.management", "", 1);
    backend.submit(vec![apk.clone()], false, false);
    wait_for(&backend, |s| s.jobs.iter().any(|j| j.stage.active()));
    backend.update_connection(endpoint.key(), Some(address(1)));
    assert!(backend.snapshot().notice.contains("仍有安装任务"));
    backend.forget_device(endpoint.key());
    assert!(backend.snapshot().notice.contains("仍有安装任务"));
    assert_eq!(storage.settings().endpoints, vec![endpoint.clone()]);
    wait_for(&backend, |s| {
        s.jobs.iter().any(|j| j.stage == JobStage::Succeeded)
    });
    assert_eq!(
        server.records.lock().expect("records").uploads,
        vec![std::fs::read(apk).expect("APK bytes")]
    );
    backend.forget_device(endpoint.key());
    wait_for(&backend, |s| {
        !s.managing.contains(&endpoint.key()) && !s.devices.iter().any(|d| d.id == endpoint.key())
    });
    assert_eq!(backend.snapshot().jobs[0].stage, JobStage::Succeeded);
}

#[test]
fn editing_a_known_address_preserves_device_identity_checks() {
    let directory = TempDirectory::new();
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let old = address(1);
    storage
        .update_settings(|s| {
            s.endpoints.push(old.clone());
            s.aliases.insert(old.key(), "原设备备注".into());
            s.device_info.insert(
                old.key(),
                DeviceInfo {
                    model: "原设备".into(),
                    serial: "EXPECTED".into(),
                    android: "16".into(),
                },
            );
        })
        .expect("saved identity");
    let server = DeviceServer::start(DeviceOptions {
        serial: "DIFFERENT",
        ..Default::default()
    });
    let backend = Backend::new(storage.clone()).expect("backend");
    wait_for(&backend, |s| {
        s.devices
            .iter()
            .any(|d| d.id == old.key() && d.status == DeviceStatus::Offline)
    });
    let updated = address(server.port);
    backend.update_connection(old.key(), Some(updated.clone()));
    wait_for(&backend, |s| {
        s.devices.iter().any(|d| {
            d.id == updated.key()
                && d.status == DeviceStatus::Offline
                && d.detail.contains("序列号")
        })
    });
    assert_eq!(
        storage.settings().device_info[&updated.key()].serial,
        "EXPECTED"
    );
    assert_eq!(storage.settings().aliases[&updated.key()], "原设备备注");
    assert!(!storage.settings().device_info.contains_key(&old.key()));
    assert!(server.records.lock().expect("records").uploads.is_empty());
}

#[test]
fn paired_connection_can_be_edited_and_reset_to_discovery_without_repairing() {
    let directory = TempDirectory::new();
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    let id = format!("adb-edit-paired-{}", std::process::id());
    let key = format!("tls:{id}");
    pair(&storage, &id);
    let server = DeviceServer::start(DeviceOptions {
        tls: true,
        idle_timeout: Duration::from_secs(25),
        ..Default::default()
    });
    let backend = Backend::new(storage.clone()).expect("backend");
    backend.update_connection(
        key.clone(),
        Some(Endpoint {
            paired_id: Some(id.clone()),
            ..address(server.port)
        }),
    );
    wait_for(&backend, |s| {
        s.devices
            .iter()
            .any(|d| d.id == key && d.status == DeviceStatus::Online)
    });
    assert_eq!(storage.paired_devices().len(), 1);
    assert_eq!(
        storage.paired_devices()[0].certificate_fingerprint,
        "fixture-only"
    );
    backend.update_connection(key.clone(), None);
    wait_for(&backend, |s| {
        s.devices
            .iter()
            .any(|d| d.id == key && d.status == DeviceStatus::Connecting && d.endpoint.is_none())
    });
    assert!(storage.settings().endpoints.is_empty());
    assert_eq!(storage.paired_devices()[0].device_id, id);
    backend.forget_device(key.clone());
    wait_for(&backend, |s| {
        !s.managing.contains(&key) && !s.devices.iter().any(|d| d.id == key)
    });
    assert!(storage.paired_devices().is_empty());
}

#[test]
fn failed_configuration_cleanup_restores_the_pairing_record() {
    let directory = TempDirectory::new();
    let storage = Storage::open(directory.0.join("data")).expect("storage");
    pair(&storage, "rollback-device");
    let id = "tls:rollback-device";
    storage
        .update_settings(|s| {
            s.aliases.insert(id.into(), "保留备注".into());
        })
        .expect("alias");
    let pairing = std::fs::read(storage.directory.join("paired.json")).expect("pairing");
    let config = std::fs::read(storage.directory.join("settings.json")).expect("config");
    std::fs::create_dir(storage.directory.join("settings.tmp")).expect("block config write");
    assert!(storage.forget_wireless(id).is_err());
    assert_eq!(
        std::fs::read(storage.directory.join("paired.json")).expect("pairing restored"),
        pairing
    );
    assert_eq!(
        std::fs::read(storage.directory.join("settings.json")).expect("settings unchanged"),
        config
    );
    assert_eq!(storage.paired_devices().len(), 1);
    assert_eq!(storage.settings().aliases[id], "保留备注");
    std::fs::remove_dir(storage.directory.join("settings.tmp")).expect("unblock config write");
    storage.forget_wireless(id).expect("retry delete");
    assert!(storage.paired_devices().is_empty());
    assert!(!storage.settings().aliases.contains_key(id));
}
