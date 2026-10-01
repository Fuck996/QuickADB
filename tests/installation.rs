mod support;
use quickadb::{
    apk::{Apk, manifest_attributes, validate_group},
    install::install,
    model::JobStage,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use support::{DeviceOptions, DeviceServer, TempDirectory};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn streaming_uses_official_arguments_and_exact_file_bytes() {
    let directory = TempDirectory::new();
    let apk = directory.apk("中文 路径.apk", 24017);
    let expected = std::fs::read(&apk.path).expect("read fixture");
    let server = DeviceServer::start(DeviceOptions::default());
    let client = server.client().await;
    let progress = Arc::new(Mutex::new(Vec::new()));
    install(
        client.clone(),
        &[apk],
        true,
        CancellationToken::new(),
        |p| progress.lock().expect("progress").push(p),
    )
    .await
    .expect("install");
    let records = server.records.lock().expect("records");
    assert_eq!(
        records.services[0],
        b"abb_exec:package\0install\0-r\0-t\0-S\024017\0"
    );
    assert_eq!(records.uploads, vec![expected]);
    drop(records);
    let states = progress.lock().expect("progress");
    assert!(
        states
            .iter()
            .any(|p| p.stage == JobStage::Transferring && p.bytes == 24017)
    );
    assert_eq!(states.last().expect("state").stage, JobStage::Installing);
    drop(states);
    client.close().await.expect("close");
}

#[tokio::test]
async fn acknowledged_bytes_control_progress_and_cancellation() {
    let directory = TempDirectory::new();
    let apk = directory.apk("test.apk", 10000);
    let server = DeviceServer::start(DeviceOptions {
        delay_ack: Duration::from_millis(140),
        ..Default::default()
    });
    let client = server.client().await;
    let progress = Arc::new(Mutex::new(Vec::new()));
    let recorder = progress.clone();
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let installed = client.clone();
    let operation = tokio::spawn(async move {
        install(installed, &[apk], false, stop, move |p| {
            recorder.lock().expect("progress").push(p)
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(65)).await;
    assert!(
        progress
            .lock()
            .expect("progress")
            .iter()
            .all(|p| p.bytes == 0)
    );
    cancel.cancel();
    let result = operation.await.expect("worker").expect_err("cancellation");
    assert_eq!(result.stage, JobStage::Canceled);
    client.close().await.expect("close");
}

#[tokio::test]
async fn connection_loss_after_final_upload_is_unknown() {
    let directory = TempDirectory::new();
    let server = DeviceServer::start(DeviceOptions {
        disconnect_after_upload: true,
        ..Default::default()
    });
    let client = server.client().await;
    let result = install(
        client,
        &[directory.apk("test.apk", 7000)],
        false,
        CancellationToken::new(),
        |_| {},
    )
    .await
    .expect_err("lost connection");
    assert_eq!(result.stage, JobStage::Unknown);
}

#[tokio::test]
async fn real_package_rejection_is_not_success_or_unknown() {
    let directory = TempDirectory::new();
    let server = DeviceServer::start(DeviceOptions {
        result: "Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE]\n",
        features: "cmd",
        ..Default::default()
    });
    let client = server.client().await;
    let result = install(
        client.clone(),
        &[directory.apk("test.apk", 100)],
        false,
        CancellationToken::new(),
        |_| {},
    )
    .await
    .expect_err("rejected APK");
    assert_eq!(result.stage, JobStage::Failed);
    assert!(result.detail.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE"));
    assert!(
        server.records.lock().expect("records").services[0]
            .starts_with(b"exec:cmd package install -r -S 100")
    );
    client.close().await.expect("close");
}

#[tokio::test]
async fn split_apks_share_a_package_session() {
    let directory = TempDirectory::new();
    let base = directory.apk("base.apk", 300);
    let mut split = directory.apk("config.apk", 500);
    split.split = "config.arm64".into();
    let server = DeviceServer::start(DeviceOptions::default());
    let client = server.client().await;
    install(
        client.clone(),
        &[base, split],
        false,
        CancellationToken::new(),
        |_| {},
    )
    .await
    .expect("split install");
    let records = server.records.lock().expect("records");
    assert_eq!(
        records.uploads.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![300, 500]
    );
    assert_eq!(records.completed, 1);
    assert!(
        records
            .services
            .last()
            .expect("commit")
            .starts_with(b"abb_exec:package\0install-commit\042\0")
    );
    drop(records);
    client.close().await.expect("close");
}

#[test]
fn manifests_and_split_group_validation_are_bounded() {
    let directory = TempDirectory::new();
    let base = Apk::read(support::write_apk(
        &directory,
        "基础.apk",
        "test.quickadb",
        "",
        4,
    ))
    .expect("base manifest");
    let split = Apk::read(support::write_apk(
        &directory,
        "拆分.apk",
        "test.quickadb",
        "config.en",
        4,
    ))
    .expect("split manifest");
    validate_group(&[base.clone(), split.clone()], true).expect("group");
    assert!(validate_group(&[split.clone()], false).is_err());
    assert!(validate_group(&[base.clone(), base], true).is_err());
    let mut bytes = support::binary_manifest("test.quickadb", "", 4);
    bytes.truncate(bytes.len() - 5);
    assert!(manifest_attributes(&bytes).is_err());
    assert!(manifest_attributes(&[0; 8]).is_err());
}

#[tokio::test]
async fn legacy_install_uses_sync_and_cleans_device_file() {
    let directory = TempDirectory::new();
    let apk = directory.apk("旧版.apk", 64023);
    let expected = std::fs::read(&apk.path).expect("read fixture");
    let server = DeviceServer::start(DeviceOptions {
        features: "",
        ..Default::default()
    });
    let client = server.client().await;
    install(
        client.clone(),
        &[apk],
        false,
        CancellationToken::new(),
        |_| {},
    )
    .await
    .expect("legacy install");
    let records = server.records.lock().expect("records");
    assert_eq!(records.uploads, vec![expected]);
    assert!(records.services[0].starts_with(b"sync:"));
    assert!(records.services[1].starts_with(b"shell:pm install -r"));
    assert!(records.services[2].starts_with(b"shell:rm -f /data/local/tmp/quickadb-"));
    drop(records);
    client.close().await.expect("close");
}

#[tokio::test]
async fn failed_split_write_abandons_the_original_session() {
    let directory = TempDirectory::new();
    let mut split = directory.apk("config.apk", 100);
    split.split = "config.en".into();
    let server = DeviceServer::start(DeviceOptions {
        result: "Failure [INSTALL_FAILED_INVALID_APK]\n",
        ..Default::default()
    });
    let client = server.client().await;
    let result = install(
        client.clone(),
        &[directory.apk("base.apk", 300), split],
        false,
        CancellationToken::new(),
        |_| {},
    )
    .await
    .expect_err("split rejected");
    assert_eq!(result.stage, JobStage::Failed);
    let records = server.records.lock().expect("records");
    assert!(
        records
            .services
            .last()
            .expect("abandon")
            .starts_with(b"abb_exec:package\0install-abandon\042\0")
    );
    assert!(
        !records
            .services
            .iter()
            .any(|s| s.starts_with(b"abb_exec:package\0install-commit"))
    );
    drop(records);
    client.close().await.expect("close");
}
