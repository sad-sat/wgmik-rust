pub mod api;
pub mod factory;
pub mod rest;
pub mod tls_setup;
pub mod version;

use api::RouterOSApiClient;
use rest::RouterOSRestClient;
use serde::{Deserialize, Serialize};

pub fn parse_last_handshake_str(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || s == "0" || s == "never" || s == "none" {
        return None;
    }
    if let Ok(num) = s.parse::<i64>() {
        return if num > 0 { Some(num) } else { None };
    }

    let mut total_seconds: i64 = 0;
    let mut current_num: i64 = 0;
    let mut has_match = false;

    for c in s.chars() {
        if c.is_ascii_digit() {
            current_num = current_num.saturating_mul(10).saturating_add(c as i64 - '0' as i64);
        } else {
            let mult = match c.to_ascii_lowercase() {
                'w' => 604_800,
                'd' => 86_400,
                'h' => 3_600,
                'm' => 60,
                's' => 1,
                _ => 0,
            };
            if mult > 0 && current_num > 0 {
                total_seconds = total_seconds.saturating_add(current_num.saturating_mul(mult));
                has_match = true;
            }
            current_num = 0;
        }
    }

    if has_match && total_seconds > 0 {
        Some(total_seconds)
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WGPeer {
    pub ros_id: String,
    pub interface: String,
    pub public_key: String,
    pub allowed_address: String,
    pub endpoint: Option<String>,
    pub current_endpoint_address: Option<String>,
    pub rx_bytes: i64,
    pub tx_bytes: i64,
    pub last_handshake: Option<i64>,
    pub disabled: bool,
    pub comment: Option<String>,
    pub name: String,
    pub client_endpoint: Option<String>,
    pub client_listen_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WGInterfaceConfig {
    pub name: String,
    pub public_key: String,
    pub listen_port: u16,
    pub private_key: Option<String>,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleQueueInfo {
    pub ros_id: String,
    pub name: String,
    pub target: String,
    pub max_limit: String,
    pub comment: Option<String>,
}

pub enum AnyRouterOSClient {
    Rest(RouterOSRestClient),
    Api(RouterOSApiClient),
}

impl AnyRouterOSClient {
    pub async fn get_system_version(&self) -> Result<String, String> {
        match self {
            Self::Rest(c) => c.get_system_version().await,
            Self::Api(c) => c.get_system_version().await,
        }
    }

    pub async fn list_wireguard_interfaces(&self) -> Result<Vec<String>, String> {
        match self {
            Self::Rest(c) => c.list_wireguard_interfaces().await,
            Self::Api(c) => c.list_wireguard_interfaces().await,
        }
    }

    pub async fn list_all_wireguard_peers(&self) -> Result<Vec<WGPeer>, String> {
        match self {
            Self::Rest(c) => c.list_all_wireguard_peers().await,
            Self::Api(c) => c.list_all_wireguard_peers().await,
        }
    }

    pub async fn list_wireguard_peers(&self, interface: &str) -> Result<Vec<WGPeer>, String> {
        match self {
            Self::Rest(c) => c.list_wireguard_peers(interface).await,
            Self::Api(c) => c.list_wireguard_peers(interface).await,
        }
    }

    pub async fn set_peer_disabled(&self, interface: &str, ros_id: &str, disabled: bool) -> Result<(), String> {
        match self {
            Self::Rest(c) => c.set_peer_disabled(interface, ros_id, disabled).await,
            Self::Api(c) => c.set_peer_disabled(interface, ros_id, disabled).await,
        }
    }

    pub async fn set_peer_keys(&self, interface: &str, ros_id: &str, public_key: &str, private_key: &str) -> Result<(), String> {
        match self {
            Self::Rest(c) => c.set_peer_keys(interface, ros_id, public_key, private_key).await,
            Self::Api(c) => c.set_peer_keys(interface, ros_id, public_key, private_key).await,
        }
    }

    pub async fn add_wireguard_peer(
        &self,
        interface: &str,
        public_key: &str,
        allowed_address: &str,
        name: &str,
        disabled: bool,
        private_key: Option<&str>,
        preshared_key: Option<&str>,
        client_endpoint: Option<&str>,
    ) -> Result<String, String> {
        match self {
            Self::Rest(c) => c.add_wireguard_peer(interface, public_key, allowed_address, name, disabled, private_key, preshared_key, client_endpoint).await,
            Self::Api(c) => c.add_wireguard_peer(interface, public_key, allowed_address, name, disabled, private_key, preshared_key, client_endpoint).await,
        }
    }

    pub async fn remove_wireguard_peer(&self, interface: &str, ros_id: &str) -> Result<(), String> {
        match self {
            Self::Rest(c) => c.remove_wireguard_peer(interface, ros_id).await,
            Self::Api(c) => c.remove_wireguard_peer(interface, ros_id).await,
        }
    }

    pub async fn get_wireguard_peer_private_key(&self, interface: &str, ros_id: &str) -> Result<Option<String>, String> {
        match self {
            Self::Rest(c) => c.get_wireguard_peer_private_key(interface, ros_id).await,
            Self::Api(c) => c.get_wireguard_peer_private_key(interface, ros_id).await,
        }
    }

    pub async fn get_wireguard_peer_preshared_key(&self, interface: &str, ros_id: &str) -> Result<Option<String>, String> {
        match self {
            Self::Rest(c) => c.get_wireguard_peer_preshared_key(interface, ros_id).await,
            Self::Api(c) => c.get_wireguard_peer_preshared_key(interface, ros_id).await,
        }
    }

    pub async fn set_peer_name(&self, interface: &str, ros_id: &str, name: &str) -> Result<(), String> {
        match self {
            Self::Rest(c) => c.set_peer_name(interface, ros_id, name).await,
            Self::Api(c) => c.set_peer_name(interface, ros_id, name).await,
        }
    }

    pub async fn set_peer_client_endpoint(&self, interface: &str, ros_id: &str, endpoint: Option<&str>) -> Result<(), String> {
        match self {
            Self::Rest(c) => c.set_peer_client_endpoint(interface, ros_id, endpoint).await,
            Self::Api(c) => c.set_peer_client_endpoint(interface, ros_id, endpoint).await,
        }
    }

    pub async fn set_peer_preshared_key(&self, interface: &str, ros_id: &str, psk: Option<&str>) -> Result<(), String> {
        match self {
            Self::Rest(c) => c.set_peer_preshared_key(interface, ros_id, psk).await,
            Self::Api(c) => c.set_peer_preshared_key(interface, ros_id, psk).await,
        }
    }

    pub async fn get_wireguard_interface(&self, interface: &str) -> Result<WGInterfaceConfig, String> {
        match self {
            Self::Rest(c) => c.get_wireguard_interface(interface).await,
            Self::Api(c) => c.get_wireguard_interface(interface).await,
        }
    }

    pub async fn get_primary_ipv4(&self) -> Result<String, String> {
        match self {
            Self::Rest(c) => c.get_primary_ipv4().await,
            Self::Api(c) => c.get_primary_ipv4().await,
        }
    }

    pub async fn add_simple_queue(&self, name: &str, target: &str, max_limit_up: &str, max_limit_down: &str, comment: &str) -> Result<String, String> {
        match self {
            Self::Rest(c) => c.add_simple_queue(name, target, max_limit_up, max_limit_down, comment).await,
            Self::Api(c) => c.add_simple_queue(name, target, max_limit_up, max_limit_down, comment).await,
        }
    }

    pub async fn update_simple_queue(&self, ros_id: &str, max_limit_up: &str, max_limit_down: &str) -> Result<(), String> {
        match self {
            Self::Rest(c) => c.update_simple_queue(ros_id, max_limit_up, max_limit_down).await,
            Self::Api(c) => c.update_simple_queue(ros_id, max_limit_up, max_limit_down).await,
        }
    }

    pub async fn set_simple_queue_name(&self, ros_id: &str, name: &str) -> Result<(), String> {
        match self {
            Self::Rest(c) => c.set_simple_queue_name(ros_id, name).await,
            Self::Api(c) => c.set_simple_queue_name(ros_id, name).await,
        }
    }

    pub async fn remove_simple_queue(&self, ros_id: &str) -> Result<(), String> {
        match self {
            Self::Rest(c) => c.remove_simple_queue(ros_id).await,
            Self::Api(c) => c.remove_simple_queue(ros_id).await,
        }
    }

    pub async fn list_simple_queues(&self, name_prefix: &str) -> Result<Vec<SimpleQueueInfo>, String> {
        match self {
            Self::Rest(c) => c.list_simple_queues(name_prefix).await,
            Self::Api(c) => c.list_simple_queues(name_prefix).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_last_handshake_str() {
        assert_eq!(parse_last_handshake_str(""), None);
        assert_eq!(parse_last_handshake_str("0"), None);
        assert_eq!(parse_last_handshake_str("never"), None);
        assert_eq!(parse_last_handshake_str("24s"), Some(24));
        assert_eq!(parse_last_handshake_str("1m30s"), Some(90));
        assert_eq!(parse_last_handshake_str("2h15m"), Some(8100));
        assert_eq!(parse_last_handshake_str("3d"), Some(259200));
        assert_eq!(parse_last_handshake_str("1w2d"), Some(777600));
        assert_eq!(parse_last_handshake_str("45"), Some(45));
    }
}
