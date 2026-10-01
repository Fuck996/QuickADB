//! mDNS discovery for Android wireless-debugging endpoints.
//!
//! Discovery is deliberately separate from authentication and connection
//! setup. A caller can inspect the returned endpoints and then choose between
//! an authorized ADB connection and a normal connection that may request user
//! approval.

use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::mpsc::{self, Receiver as StdReceiver, Sender},
    time::{Duration, Instant},
};

use mdns_sd::{Receiver as MdnsReceiver, ResolvedService, ScopedIp, ServiceDaemon, ServiceEvent};
use thiserror::Error;

/// mDNS service advertised by Android wireless debugging for ADB connections.
pub const ADB_TLS_CONNECT_SERVICE: &str = "_adb-tls-connect._tcp.local.";

/// Pairing service published while the phone's pairing dialog is open.
pub const ADB_TLS_PAIRING_SERVICE: &str = "_adb-tls-pairing._tcp.local.";

/// Legacy mDNS service advertised by adbd TCP endpoints.
pub const ADB_LEGACY_SERVICE: &str = "_adb._tcp.local.";

const ADB_SERVICES: [&str; 3] = [
    ADB_TLS_CONNECT_SERVICE,
    ADB_TLS_PAIRING_SERVICE,
    ADB_LEGACY_SERVICE,
];

/// Protocol advertised by a wireless debugging service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdbServiceType {
    /// TLS debugging connection, requiring an existing pairing credential.
    TlsConnect,
    /// Pairing server accepting the code shown on the phone.
    Pairing,
    /// Traditional ADB TCP connection.
    Legacy,
}

/// Discovery changes, including services withdrawn by the device.
#[derive(Debug, Clone)]
pub enum AdbMdnsEvent {
    /// Service resolved or its advertised metadata changed.
    Resolved(AdbMdnsDevice),
    /// Fully qualified name of a withdrawn service.
    Removed(String),
}

/// Errors returned by mDNS discovery.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MdnsDiscoveryError {
    /// The mDNS daemon could not be created or controlled.
    #[error("mDNS discovery failed: {0}")]
    Daemon(#[from] mdns_sd::Error),

    /// The discovery worker could not be joined.
    #[error("mDNS discovery worker failed: {0}")]
    Worker(String),
}

/// A wireless ADB endpoint discovered through mDNS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbMdnsDevice {
    /// Fully qualified mDNS service name.
    pub fullname: String,
    /// Service instance name, distinct from the phone's product model.
    pub instance_name: String,
    /// Product model from Android's `name` TXT property, when advertised.
    pub model: Option<String>,
    /// Connection or pairing protocol advertised by this service.
    pub service_type: AdbServiceType,
    /// Resolved addresses for the service.
    addresses: BTreeSet<IpAddr>,
    /// TCP port advertised by the service.
    pub port: u16,
}

impl AdbMdnsDevice {
    fn from_service(service: ResolvedService) -> Option<Self> {
        let port = service.port;
        if port == 0 {
            return None;
        }
        let service_type = match service.ty_domain.as_str() {
            ADB_TLS_CONNECT_SERVICE => AdbServiceType::TlsConnect,
            ADB_TLS_PAIRING_SERVICE => AdbServiceType::Pairing,
            ADB_LEGACY_SERVICE => AdbServiceType::Legacy,
            _ => return None,
        };
        let instance_name = service
            .fullname
            .strip_suffix(&format!(".{}", service.ty_domain))?
            .to_owned();
        let model = service
            .get_property_val_str("name")
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        Some(Self {
            fullname: service.fullname,
            instance_name,
            model,
            service_type,
            addresses: service.addresses.iter().map(ScopedIp::to_ip_addr).collect(),
            port,
        })
    }

    /// Returns all resolved addresses.
    #[must_use]
    pub fn addresses(&self) -> BTreeSet<IpAddr> {
        self.addresses.clone()
    }

    /// Returns all resolved IPv4 addresses.
    #[must_use]
    pub fn ipv4_addresses(&self) -> BTreeSet<Ipv4Addr> {
        self.addresses
            .iter()
            .filter_map(|address| match address {
                IpAddr::V4(address) => Some(*address),
                IpAddr::V6(_) => None,
            })
            .collect()
    }

    /// Returns all resolved IPv6 addresses.
    #[must_use]
    pub fn ipv6_addresses(&self) -> BTreeSet<Ipv6Addr> {
        self.addresses
            .iter()
            .filter_map(|address| match address {
                IpAddr::V4(_) => None,
                IpAddr::V6(address) => Some(*address),
            })
            .collect()
    }
}

/// A running mDNS browser for wireless ADB endpoints.
pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
}

impl std::fmt::Debug for MdnsDiscovery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MdnsDiscovery")
            .field("metrics", &self.daemon.get_metrics())
            .finish()
    }
}

impl MdnsDiscovery {
    /// Creates an mDNS discovery daemon.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform mDNS daemon cannot be initialized.
    pub fn new() -> Result<Self, MdnsDiscoveryError> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
        })
    }

    /// Starts browsing modern and legacy ADB mDNS services.
    ///
    /// # Errors
    ///
    /// Returns an error when the service browser or forwarding worker cannot
    /// be started.
    pub fn browse(&self) -> Result<StdReceiver<AdbMdnsEvent>, MdnsDiscoveryError> {
        let events = ADB_SERVICES
            .iter()
            .map(|service| self.daemon.browse(service))
            .collect::<Result<Vec<_>, _>>()?;
        let (sender, receiver) = mpsc::channel();
        for (index, events) in events.into_iter().enumerate() {
            let sender = sender.clone();
            std::thread::Builder::new()
                .name(format!("droidmux-mdns-{index}"))
                .spawn(move || forward_resolved_events(events, sender))
                .map_err(|error| MdnsDiscoveryError::Worker(error.to_string()))?;
        }
        Ok(receiver)
    }

    /// Discovers all endpoints observed during `timeout`.
    ///
    /// # Errors
    ///
    /// Returns an error when mDNS initialization, browsing, or the worker
    /// fails.
    pub async fn discover_once(
        timeout: Duration,
    ) -> Result<Vec<AdbMdnsDevice>, MdnsDiscoveryError> {
        let discovery = Self::new()?;
        let receiver = discovery.browse()?;
        let result = tokio::task::spawn_blocking(move || {
            let mut devices = BTreeMap::new();
            let started = Instant::now();
            loop {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    break;
                }
                match receiver.recv_timeout(remaining) {
                    Ok(AdbMdnsEvent::Resolved(device)) => {
                        devices.insert(device.fullname.clone(), device);
                    }
                    Ok(AdbMdnsEvent::Removed(fullname)) => {
                        devices.remove(&fullname);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                        break;
                    }
                }
            }
            devices.into_values().collect()
        })
        .await
        .map_err(|error| MdnsDiscoveryError::Worker(error.to_string()))?;
        discovery.shutdown()?;
        Ok(result)
    }

    /// Stops the mDNS daemon and its browser.
    ///
    /// # Errors
    ///
    /// Returns an error when the mDNS daemon cannot be shut down.
    pub fn shutdown(&self) -> Result<(), MdnsDiscoveryError> {
        self.daemon.shutdown().map(|_| ()).map_err(Into::into)
    }
}

impl Drop for MdnsDiscovery {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

#[allow(clippy::needless_pass_by_value)]
fn forward_resolved_events(events: MdnsReceiver<ServiceEvent>, sender: Sender<AdbMdnsEvent>) {
    while let Ok(event) = events.recv() {
        let change = match event {
            ServiceEvent::ServiceResolved(service) => {
                AdbMdnsDevice::from_service(*service).map(AdbMdnsEvent::Resolved)
            }
            ServiceEvent::ServiceRemoved(_, fullname) => Some(AdbMdnsEvent::Removed(fullname)),
            _ => None,
        };
        if let Some(change) = change
            && sender.send(change).is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_resolved_address_families() {
        let device = AdbMdnsDevice {
            fullname: "adb-test._adb-tls-connect._tcp.local.".to_owned(),
            instance_name: "adb-test".to_owned(),
            model: None,
            service_type: AdbServiceType::TlsConnect,
            addresses: [
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ]
            .into_iter()
            .collect(),
            port: 37_001,
        };

        assert_eq!(
            device.ipv4_addresses(),
            [Ipv4Addr::LOCALHOST].into_iter().collect()
        );
        assert_eq!(
            device.ipv6_addresses(),
            [Ipv6Addr::LOCALHOST].into_iter().collect()
        );
    }

    #[test]
    fn uses_android_wireless_debugging_service_name() {
        assert_eq!(ADB_TLS_CONNECT_SERVICE, "_adb-tls-connect._tcp.local.");
        assert_eq!(ADB_TLS_PAIRING_SERVICE, "_adb-tls-pairing._tcp.local.");
        assert_eq!(ADB_LEGACY_SERVICE, "_adb._tcp.local.");
    }

    #[test]
    fn resolved_service_preserves_product_model_and_pairing_port() {
        let service = mdns_sd::ServiceInfo::new(
            ADB_TLS_PAIRING_SERVICE,
            "adb-test-pair",
            "android.local.",
            "192.168.1.8",
            37_002,
            [("name", "Pixel 8"), ("v", "1")].as_slice(),
        )
        .expect("pairing advertisement")
        .as_resolved_service();
        let device = AdbMdnsDevice::from_service(service).expect("resolved pairing service");
        assert_eq!(device.model.as_deref(), Some("Pixel 8"));
        assert_eq!(device.instance_name, "adb-test-pair");
        assert_eq!(device.service_type, AdbServiceType::Pairing);
        assert_eq!(device.port, 37_002);
    }

    #[test]
    fn service_instance_is_retained_without_a_product_model() {
        let service = mdns_sd::ServiceInfo::new(
            ADB_TLS_CONNECT_SERVICE,
            "adb-test-connect",
            "android.local.",
            "192.168.1.8",
            37_003,
            [("v", "1")].as_slice(),
        )
        .expect("connection advertisement")
        .as_resolved_service();
        let device = AdbMdnsDevice::from_service(service).expect("resolved connection service");
        assert_eq!(device.model, None);
        assert_eq!(device.instance_name, "adb-test-connect");
        assert_eq!(device.service_type, AdbServiceType::TlsConnect);
        assert_eq!(device.port, 37_003);
    }
}
