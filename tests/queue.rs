mod support;
use quickadb::{
    engine::Backend,
    model::{DeviceStatus, Endpoint, JobStage},
    storage::Storage,
};
use std::time::{Duration, Instant};
use support::{DeviceOptions, DeviceServer, TempDirectory};

fn wait_for(backend: &Backend, condition: impl Fn(&quickadb::model::Snapshot) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
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
    for device in &devices {
        backend.toggle(&device.id);
    }
    let first = support::write_apk(&directory, "中文 第一.apk", "test.first", "", 1);
    let second = support::write_apk(&directory, "第二.apk", "test.second", "", 2);
    backend.submit(vec![first.clone(), second.clone()], false, false);
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
