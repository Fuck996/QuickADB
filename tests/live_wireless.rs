// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use droidmux::{
    client::AdbClient,
    discovery::{AdbMdnsEvent, AdbServiceType, MdnsDiscovery},
    pairing::{WirelessTransport, WirelessTransportConfig},
    protocol::AdbPacket,
    transport::{AdbTransport, AdbTransportError},
};
use quickadb::{engine::shell_text, storage::Storage};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct TraceTransport {
    transport: WirelessTransport,
    trace: Arc<Mutex<VecDeque<String>>>,
}

impl TraceTransport {
    fn record(&self, direction: &str, packet: &AdbPacket) {
        let mut trace = self.trace.lock().expect("trace lock");
        if trace.len() == 32 {
            trace.pop_front();
        }
        trace.push_back(format!(
            "{direction} {:?} {}:{} {} bytes",
            packet.command,
            packet.arg0,
            packet.arg1,
            packet.payload.len()
        ));
    }
}

#[async_trait]
impl AdbTransport for TraceTransport {
    async fn read_packet(&mut self) -> Result<AdbPacket, AdbTransportError> {
        let packet = self.transport.read_packet().await?;
        self.record("receive", &packet);
        Ok(packet)
    }
    async fn write_packet(&mut self, packet: &AdbPacket) -> Result<(), AdbTransportError> {
        self.record("send", packet);
        self.transport.write_packet(packet).await
    }
    async fn close(&mut self) -> Result<(), AdbTransportError> {
        self.transport.close().await
    }
    fn peer_description(&self) -> String {
        self.transport.peer_description()
    }
}

// 显式指定已配对 GUID 后才运行；只查询属性并向 /dev/null 发送数据，不安装应用。
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires QUICKADB_LIVE_ID and an awake, paired Android device"]
async fn wireless_shell_and_transfer_diagnostic() -> Result<()> {
    let id = std::env::var("QUICKADB_LIVE_ID").context("set QUICKADB_LIVE_ID")?;
    let data = std::path::PathBuf::from(std::env::var("LOCALAPPDATA")?).join("QuickADB");
    ensure!(
        data.join("host.key").is_file(),
        "existing credential required"
    );
    let storage = Storage::open(data)?;
    ensure!(
        storage.paired_devices().iter().any(|p| p.device_id == id),
        "paired GUID required"
    );
    let discovery = MdnsDiscovery::new()?;
    let receiver = discovery.browse()?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut found = None;
    while Instant::now() < deadline {
        while let Ok(event) = receiver.try_recv() {
            if let AdbMdnsEvent::Resolved(device) = event
                && device.service_type == AdbServiceType::TlsConnect
                && device.instance_name == id
            {
                found = Some(device);
            }
        }
        if found.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    discovery.shutdown()?;
    let device = found.context("no current mDNS connection service for target")?;
    let host = device
        .ipv4_addresses()
        .first()
        .context("no IPv4 address")?
        .to_string();
    let credential = storage.credential()?;
    let transport = WirelessTransport::connect(
        host,
        device.port,
        &credential,
        WirelessTransportConfig {
            connect_timeout: Duration::from_secs(5),
            ..Default::default()
        },
    )
    .await?;
    let trace = Arc::new(Mutex::new(VecDeque::new()));
    let client = AdbClient::connect(
        Box::new(TraceTransport {
            transport,
            trace: trace.clone(),
        }),
        credential,
    )
    .await?;
    let result = async {
        let mut failures = 0;
        for _ in 0..30 {
            if let Err(error) = shell_text(&client, "echo quickadb").await {
                failures += 1;
                eprintln!(
                    "shell failure: {error:#}\n{}",
                    trace
                        .lock()
                        .expect("trace")
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
        eprintln!("shell failures: {failures}/30");
        let stream = client
            .open_service("exec:cat > /dev/null; echo quickadb")
            .await?;
        let start = Instant::now();
        let chunk = vec![0_u8; stream.max_payload().min(256 * 1024)];
        for _ in 0..32 {
            tokio::time::timeout(Duration::from_secs(30), stream.write(chunk.clone())).await??;
        }
        eprintln!(
            "confirmed transfer: {} bytes, {:.3}s, {:.2} MiB/s",
            chunk.len() * 32,
            start.elapsed().as_secs_f64(),
            (chunk.len() * 32) as f64 / 1048576.0 / start.elapsed().as_secs_f64()
        );
        stream.close().await?;
        ensure!(failures == 0, "shell failed {failures}/30 times");
        Ok::<_, anyhow::Error>(())
    }
    .await;
    client.close().await?;
    result
}
