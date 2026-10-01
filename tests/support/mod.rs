use bytes::Bytes;
use droidmux::{
    auth::RsaAdbCredential,
    client::AdbClient,
    protocol::{ADB_VERSION, AdbCommand, AdbHeader, AdbPacket},
    tcp::{TcpTransport, TcpTransportConfig},
};
use quickadb::apk::Apk;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime},
};

pub struct TempDirectory(pub PathBuf);
impl TempDirectory {
    pub fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("quickadb-tests-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create fixture directory");
        Self(directory)
    }
    pub fn apk(&self, name: &str, length: usize) -> Apk {
        let path = self.0.join(name);
        let bytes: Vec<_> = (0..length).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, bytes).expect("write fixture");
        let metadata = std::fs::metadata(&path).expect("metadata");
        Apk {
            path,
            size: metadata.len(),
            modified: metadata.modified().expect("mtime"),
            package: "test.quickadb".into(),
            version: "0:1".into(),
            split: String::new(),
        }
    }
}
impl Drop for TempDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove fixture directory");
    }
}

#[derive(Clone)]
pub struct DeviceOptions {
    pub features: &'static str,
    pub result: &'static str,
    pub delay_ack: Duration,
    pub disconnect_after_upload: bool,
    pub serial: &'static str,
}
impl Default for DeviceOptions {
    fn default() -> Self {
        Self {
            features: "cmd,abb_exec",
            result: "Success\n",
            delay_ack: Duration::ZERO,
            disconnect_after_upload: false,
            serial: "QUICKADB-TEST",
        }
    }
}

#[derive(Default)]
pub struct Records {
    pub services: Vec<Vec<u8>>,
    pub uploads: Vec<Vec<u8>>,
    pub completed: usize,
}

pub struct DeviceServer {
    pub port: u16,
    pub records: Arc<Mutex<Records>>,
    thread: Option<thread::JoinHandle<()>>,
}

struct StreamState {
    expected: usize,
    payload: Vec<u8>,
    pending_close: bool,
    sync: bool,
    sync_buffer: Vec<u8>,
}

impl DeviceServer {
    pub fn start(options: DeviceOptions) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        let port = listener.local_addr().expect("address").port();
        let records = Arc::new(Mutex::new(Records::default()));
        let copy = records.clone();
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept fixture connection");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("socket timeout");
            socket.set_nodelay(true).expect("socket no delay");
            let handshake = read_packet(&mut socket).expect("handshake");
            assert_eq!(handshake.command, AdbCommand::Connect);
            let banner = format!(
                "device::features={};product=protocol-test\0",
                options.features
            );
            send(
                &mut socket,
                AdbCommand::Connect,
                ADB_VERSION,
                4096,
                banner.as_bytes(),
            );
            let mut streams = BTreeMap::<u32, StreamState>::new();
            while let Ok(packet) = read_packet(&mut socket) {
                let local = packet.arg0;
                let remote = local + 100;
                match packet.command {
                    AdbCommand::Open => {
                        copy.lock()
                            .expect("records")
                            .services
                            .push(packet.payload.to_vec());
                        let text = String::from_utf8_lossy(&packet.payload);
                        send(&mut socket, AdbCommand::Okay, remote, local, &[]);
                        let parts: Vec<_> =
                            text.trim_end_matches('\0').split(['\0', ' ']).collect();
                        let mut state = StreamState {
                            expected: 0,
                            payload: Vec::new(),
                            pending_close: false,
                            sync: text.starts_with("sync:"),
                            sync_buffer: Vec::new(),
                        };
                        if text.starts_with("shell:") {
                            let reply = if text.contains("getprop") {
                                format!("Protocol Test Device\n{}\n14\n", options.serial)
                            } else if text.starts_with("shell:pm install ") {
                                options.result.into()
                            } else if text.starts_with("shell:rm -f ") {
                                String::new()
                            } else {
                                "quickadb\n".into()
                            };
                            if !reply.is_empty() {
                                send(
                                    &mut socket,
                                    AdbCommand::Write,
                                    remote,
                                    local,
                                    reply.as_bytes(),
                                );
                                state.pending_close = true;
                            } else {
                                send(&mut socket, AdbCommand::Close, remote, local, &[]);
                            }
                        } else if state.sync {
                            state.expected = usize::MAX;
                        } else if parts.iter().any(|p| p.ends_with("install-create")) {
                            send(
                                &mut socket,
                                AdbCommand::Write,
                                remote,
                                local,
                                b"Success: created install session [42]\n",
                            );
                            state.pending_close = true;
                        } else if parts.iter().any(|p| {
                            p.ends_with("install-commit") || p.ends_with("install-abandon")
                        }) {
                            send(
                                &mut socket,
                                AdbCommand::Write,
                                remote,
                                local,
                                if parts.iter().any(|p| p.ends_with("install-abandon")) {
                                    b"Success\n"
                                } else {
                                    options.result.as_bytes()
                                },
                            );
                            state.pending_close = true;
                            if parts.iter().any(|p| p.ends_with("install-commit")) {
                                copy.lock().expect("records").completed += 1;
                            }
                        } else {
                            let size_index = parts
                                .iter()
                                .position(|p| *p == "-S")
                                .expect("installation has -S");
                            state.expected =
                                parts[size_index + 1].parse().expect("installation size");
                        }
                        streams.insert(local, state);
                    }
                    AdbCommand::Write => {
                        let state = streams.get_mut(&local).expect("known stream");
                        if state.sync {
                            state.sync_buffer.extend_from_slice(&packet.payload);
                            send(&mut socket, AdbCommand::Okay, remote, local, &[]);
                            while state.sync_buffer.len() >= 8 {
                                let command = state.sync_buffer[..4].to_vec();
                                let length = u32::from_le_bytes(
                                    state.sync_buffer[4..8].try_into().expect("sync header"),
                                ) as usize;
                                if command == b"DONE" {
                                    state.sync_buffer.drain(..8);
                                    copy.lock()
                                        .expect("records")
                                        .uploads
                                        .push(state.payload.clone());
                                    send(
                                        &mut socket,
                                        AdbCommand::Write,
                                        remote,
                                        local,
                                        b"OKAY\0\0\0\0",
                                    );
                                } else {
                                    if state.sync_buffer.len() < 8 + length {
                                        break;
                                    }
                                    assert!(command == b"SEND" || command == b"DATA");
                                    if command == b"DATA" {
                                        state
                                            .payload
                                            .extend_from_slice(&state.sync_buffer[8..8 + length]);
                                    }
                                    state.sync_buffer.drain(..8 + length);
                                }
                            }
                            continue;
                        }
                        state.payload.extend_from_slice(&packet.payload);
                        thread::sleep(options.delay_ack);
                        if options.disconnect_after_upload && state.payload.len() == state.expected
                        {
                            break;
                        }
                        send(&mut socket, AdbCommand::Okay, remote, local, &[]);
                        if state.payload.len() == state.expected {
                            copy.lock()
                                .expect("records")
                                .uploads
                                .push(state.payload.clone());
                            send(
                                &mut socket,
                                AdbCommand::Write,
                                remote,
                                local,
                                options.result.as_bytes(),
                            );
                            state.pending_close = true;
                        }
                    }
                    AdbCommand::Okay => {
                        if streams.get(&local).is_some_and(|s| s.pending_close) {
                            streams.remove(&local);
                            send(&mut socket, AdbCommand::Close, remote, local, &[]);
                        }
                    }
                    AdbCommand::Close => {
                        streams.remove(&local);
                    }
                    other => panic!("unexpected fixture packet {other:?}"),
                }
            }
        });
        Self {
            port,
            records,
            thread: Some(worker),
        }
    }

    pub async fn client(&self) -> Arc<AdbClient> {
        let credential = Arc::new(RsaAdbCredential::generate("test").expect("fixture RSA"));
        let transport = TcpTransport::connect(
            ([127, 0, 0, 1], self.port).into(),
            TcpTransportConfig::default(),
        )
        .await
        .expect("fixture TCP");
        Arc::new(
            AdbClient::connect(Box::new(transport), credential)
                .await
                .expect("fixture ADB"),
        )
    }
}

impl Drop for DeviceServer {
    fn drop(&mut self) {
        if let Some(worker) = self.thread.take() {
            worker.join().expect("fixture worker failed");
        }
    }
}

fn read_packet(socket: &mut TcpStream) -> std::io::Result<AdbPacket> {
    let mut header = [0u8; 24];
    socket.read_exact(&mut header)?;
    let parsed = AdbHeader::decode(&header).map_err(std::io::Error::other)?;
    let mut bytes = header.to_vec();
    bytes.resize(24 + parsed.payload_length as usize, 0);
    socket.read_exact(&mut bytes[24..])?;
    AdbPacket::decode(&bytes)
        .map(|p| p.0)
        .map_err(std::io::Error::other)
}

fn send(socket: &mut TcpStream, command: AdbCommand, arg0: u32, arg1: u32, bytes: &[u8]) {
    let packet =
        AdbPacket::new(command, arg0, arg1, Bytes::copy_from_slice(bytes)).expect("fixture packet");
    socket
        .write_all(&packet.encode().expect("fixture frame"))
        .expect("fixture write");
}

pub fn binary_manifest(package: &str, split: &str, version: u32) -> Vec<u8> {
    let strings = [
        "manifest",
        "package",
        package,
        "versionCode",
        "split",
        split,
    ];
    let mut pool = Vec::new();
    let mut offsets = Vec::new();
    for text in &strings {
        offsets.push(pool.len() as u32);
        pool.push(text.len() as u8);
        pool.push(text.len() as u8);
        pool.extend_from_slice(text.as_bytes());
        pool.push(0);
    }
    while pool.len() % 4 != 0 {
        pool.push(0);
    }
    let start = 28 + offsets.len() * 4;
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&1u16.to_le_bytes());
    chunk.extend_from_slice(&28u16.to_le_bytes());
    chunk.extend_from_slice(&((start + pool.len()) as u32).to_le_bytes());
    chunk.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    chunk.extend_from_slice(&0u32.to_le_bytes());
    chunk.extend_from_slice(&0x100u32.to_le_bytes());
    chunk.extend_from_slice(&(start as u32).to_le_bytes());
    chunk.extend_from_slice(&0u32.to_le_bytes());
    for offset in offsets {
        chunk.extend_from_slice(&offset.to_le_bytes());
    }
    chunk.extend_from_slice(&pool);
    let mut root = Vec::new();
    root.extend_from_slice(&0x102u16.to_le_bytes());
    root.extend_from_slice(&16u16.to_le_bytes());
    root.extend_from_slice(&96u32.to_le_bytes());
    root.extend_from_slice(&1u32.to_le_bytes());
    root.extend_from_slice(&u32::MAX.to_le_bytes());
    root.extend_from_slice(&u32::MAX.to_le_bytes());
    root.extend_from_slice(&0u32.to_le_bytes());
    root.extend_from_slice(&20u16.to_le_bytes());
    root.extend_from_slice(&20u16.to_le_bytes());
    root.extend_from_slice(&3u16.to_le_bytes());
    root.extend_from_slice(&[0; 6]);
    for (name, raw, kind, value) in [
        (1u32, 2u32, 3u8, 2u32),
        (3, u32::MAX, 0x10, version),
        (4, 5, 3, 5),
    ] {
        root.extend_from_slice(&u32::MAX.to_le_bytes());
        root.extend_from_slice(&name.to_le_bytes());
        root.extend_from_slice(&raw.to_le_bytes());
        root.extend_from_slice(&8u16.to_le_bytes());
        root.push(0);
        root.push(kind);
        root.extend_from_slice(&value.to_le_bytes());
    }
    let mut result = Vec::new();
    result.extend_from_slice(&3u16.to_le_bytes());
    result.extend_from_slice(&8u16.to_le_bytes());
    result.extend_from_slice(&((8 + chunk.len() + root.len()) as u32).to_le_bytes());
    result.extend_from_slice(&chunk);
    result.extend_from_slice(&root);
    result
}

pub fn write_apk(
    directory: &TempDirectory,
    name: &str,
    package: &str,
    split: &str,
    version: u32,
) -> PathBuf {
    let path = directory.0.join(name);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).expect("ZIP file"));
    zip.start_file(
        "AndroidManifest.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("ZIP entry");
    zip.write_all(&binary_manifest(package, split, version))
        .expect("ZIP manifest");
    zip.finish().expect("ZIP complete");
    path
}
