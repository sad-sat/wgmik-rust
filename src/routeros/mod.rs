pub mod api;
pub mod factory;
pub mod rest;
pub mod tls_setup;
pub mod version;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WGPeer {
    pub ros_id: String,
    pub interface: String,
    pub name: String,
    pub public_key: String,
    pub allowed_address: String,
    pub disabled: bool,
    pub rx_bytes: i64,
    pub tx_bytes: i64,
    pub last_handshake: Option<i64>, // epoch seconds
    pub endpoint: String,
    pub client_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WGInterfaceConfig {
    pub name: String,
    pub public_key: String,
    pub listen_port: u16,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleQueueInfo {
    pub ros_id: String,
    pub name: String,
    pub target: String,
    pub max_limit: String,
    pub comment: String,
}

#[async_trait]
pub trait RouterOSClient: Send + Sync {
    async fn get_system_version(&self) -> Result<String, String>;
    async fn list_wireguard_interfaces(&self) -> Result<Vec<String>, String>;
    async fn list_all_wireguard_peers(&self) -> Result<Vec<WGPeer>, String>;
    async fn list_wireguard_peers(&self, interface: &str) -> Result<Vec<WGPeer>, String>;
    async fn set_peer_disabled(&self, interface: &str, ros_id: &str, disabled: bool) -> Result<(), String>;
    async fn set_peer_keys(&self, interface: &str, ros_id: &str, public_key: &str, private_key: &str) -> Result<(), String>;
    async fn add_wireguard_peer(
        &self,
        interface: &str,
        public_key: &str,
        allowed_address: &str,
        name: &str,
        disabled: bool,
        private_key: Option<&str>,
        preshared_key: Option<&str>,
        client_endpoint: Option<&str>,
    ) -> Result<String, String>;
    async fn remove_wireguard_peer(&self, interface: &str, ros_id: &str) -> Result<(), String>;
    async fn get_wireguard_peer_private_key(&self, interface: &str, ros_id: &str) -> Result<Option<String>, String>;
    async fn get_wireguard_peer_preshared_key(&self, interface: &str, ros_id: &str) -> Result<Option<String>, String>;
    async fn set_peer_name(&self, interface: &str, ros_id: &str, name: &str) -> Result<(), String>;
    async fn set_peer_client_endpoint(&self, interface: &str, ros_id: &str, client_endpoint: Option<&str>) -> Result<(), String>;
    async fn set_peer_preshared_key(&self, interface: &str, ros_id: &str, preshared_key: Option<&str>) -> Result<(), String>;
    async fn get_wireguard_interface(&self, interface: &str) -> Result<WGInterfaceConfig, String>;
    async fn get_primary_ipv4(&self) -> Result<String, String>;

    // Simple Queues
    async fn add_simple_queue(&self, name: &str, target: &str, max_limit_up: &str, max_limit_down: &str, comment: &str) -> Result<String, String>;
    async fn update_simple_queue(&self, ros_id: &str, max_limit_up: &str, max_limit_down: &str) -> Result<(), String>;
    async fn set_simple_queue_name(&self, ros_id: &str, name: &str) -> Result<(), String>;
    async fn remove_simple_queue(&self, ros_id: &str) -> Result<(), String>;
    async fn list_simple_queues(&self, name_prefix: &str) -> Result<Vec<SimpleQueueInfo>, String>;
}
