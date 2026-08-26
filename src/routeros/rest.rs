use super::{SimpleQueueInfo, WGInterfaceConfig, WGPeer};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use std::time::Duration;

pub struct RouterOSRestClient {
    client: Client,
    host: String,
    port: u16,
    username: String,
    password: String,
    https: bool,
    allow_scheme_fallback: bool,
}

impl RouterOSRestClient {
    pub fn new(
        host: String,
        port: u16,
        username: String,
        password: String,
        tls_verify: bool,
        https: bool,
        allow_scheme_fallback: bool,
        timeout_duration: Duration,
    ) -> Self {
        let client = Client::builder()
            .timeout(timeout_duration)
            .danger_accept_invalid_certs(!tls_verify)
            .build()
            .unwrap();

        Self {
            client,
            host,
            port,
            username,
            password,
            https,
            allow_scheme_fallback,
        }
    }

    fn url_for(&self, path: &str, https: bool) -> String {
        let scheme = if https { "https" } else { "http" };
        format!("{}://{}:{}/rest/{}", scheme, self.host, self.port, path.trim_start_matches('/'))
    }

    async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let mut last_err = String::new();
        let schemes = if self.allow_scheme_fallback {
            vec![self.https, !self.https]
        } else {
            vec![self.https]
        };

        for is_https in schemes {
            let url = self.url_for(path, is_https);
            let mut req = self.client.request(method.clone(), &url)
                .basic_auth(&self.username, Some(&self.password));

            if let Some(ref b) = body {
                req = req.json(b);
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        if let Ok(v) = resp.json::<Value>().await {
                            return Ok(v);
                        }
                        return Ok(json!({}));
                    } else {
                        let text = resp.text().await.unwrap_or_default();
                        last_err = format!("RouterOS HTTP {}: {}", status, text);
                    }
                }
                Err(e) => {
                    last_err = format!("Connection failed to {}: {}", url, e);
                }
            }
        }

        Err(last_err)
    }

    pub async fn get_system_version(&self) -> Result<String, String> {
        let res = self.request_json(Method::GET, "system/resource", None).await?;
        if let Some(v) = res.get("version").and_then(|v| v.as_str()) {
            return Ok(v.to_string());
        }
        if let Some(arr) = res.as_array() {
            if let Some(first) = arr.first() {
                if let Some(v) = first.get("version").and_then(|v| v.as_str()) {
                    return Ok(v.to_string());
                }
            }
        }
        Err("Could not retrieve system resource version".to_string())
    }

    pub async fn list_wireguard_interfaces(&self) -> Result<Vec<String>, String> {
        let res = self.request_json(Method::GET, "interface/wireguard", None).await?;
        let mut ifaces = Vec::new();
        if let Some(arr) = res.as_array() {
            for item in arr {
                if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                    ifaces.push(name.to_string());
                }
            }
        }
        Ok(ifaces)
    }

    pub async fn list_all_wireguard_peers(&self) -> Result<Vec<WGPeer>, String> {
        let res = self.request_json(Method::GET, "interface/wireguard/peers", None).await?;
        let mut peers = Vec::new();

        if let Some(arr) = res.as_array() {
            for item in arr {
                let ros_id = item.get(".id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let interface = item.get("interface").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let public_key = item.get("public-key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let allowed_address = item.get("allowed-address").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let disabled = item.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
                let rx_bytes = item.get("rx").and_then(|v| v.as_i64()).unwrap_or(0);
                let tx_bytes = item.get("tx").and_then(|v| v.as_i64()).unwrap_or(0);
                let endpoint = item.get("endpoint-address").and_then(|v| v.as_str()).map(|s| s.to_string());
                let client_endpoint = item.get("client-endpoint").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();

                peers.push(WGPeer {
                    ros_id,
                    interface,
                    name,
                    public_key,
                    allowed_address,
                    endpoint,
                    current_endpoint_address: None,
                    rx_bytes,
                    tx_bytes,
                    last_handshake: None,
                    disabled,
                    comment: None,
                    client_endpoint: if client_endpoint.is_empty() { None } else { Some(client_endpoint) },
                    client_listen_port: None,
                });
            }
        }
        Ok(peers)
    }

    pub async fn list_wireguard_peers(&self, interface: &str) -> Result<Vec<WGPeer>, String> {
        let all = self.list_all_wireguard_peers().await?;
        Ok(all.into_iter().filter(|p| p.interface == interface).collect())
    }

    pub async fn set_peer_disabled(&self, _interface: &str, ros_id: &str, disabled: bool) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "disabled": if disabled { "yes" } else { "no" }
        });
        self.request_json(Method::POST, "interface/wireguard/peers/set", Some(body)).await?;
        Ok(())
    }

    pub async fn set_peer_keys(&self, _interface: &str, ros_id: &str, public_key: &str, private_key: &str) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "public-key": public_key,
            "private-key": private_key
        });
        self.request_json(Method::POST, "interface/wireguard/peers/set", Some(body)).await?;
        Ok(())
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
        let mut body_map = serde_json::Map::new();
        body_map.insert("interface".to_string(), json!(interface));
        body_map.insert("public-key".to_string(), json!(public_key));
        body_map.insert("allowed-address".to_string(), json!(allowed_address));
        body_map.insert("name".to_string(), json!(name));
        body_map.insert("disabled".to_string(), json!(if disabled { "yes" } else { "no" }));

        if let Some(priv_k) = private_key {
            if !priv_k.is_empty() {
                body_map.insert("private-key".to_string(), json!(priv_k));
            }
        }
        if let Some(psk) = preshared_key {
            if !psk.is_empty() {
                body_map.insert("preshared-key".to_string(), json!(psk));
            }
        }
        if let Some(ep) = client_endpoint {
            if !ep.is_empty() {
                body_map.insert("client-endpoint".to_string(), json!(ep));
            }
        }

        let res = self.request_json(Method::PUT, "interface/wireguard/peers", Some(Value::Object(body_map))).await?;
        if let Some(ret) = res.get("ret").and_then(|v| v.as_str()) {
            return Ok(ret.to_string());
        }
        if let Some(id) = res.get(".id").and_then(|v| v.as_str()) {
            return Ok(id.to_string());
        }
        Ok(String::new())
    }

    pub async fn remove_wireguard_peer(&self, _interface: &str, ros_id: &str) -> Result<(), String> {
        let path = format!("interface/wireguard/peers/{}", ros_id);
        self.request_json(Method::DELETE, &path, None).await?;
        Ok(())
    }

    pub async fn get_wireguard_peer_private_key(&self, _interface: &str, ros_id: &str) -> Result<Option<String>, String> {
        let path = format!("interface/wireguard/peers/{}", ros_id);
        let res = self.request_json(Method::GET, &path, None).await?;
        Ok(res.get("private-key").and_then(|v| v.as_str()).map(|s| s.to_string()))
    }

    pub async fn get_wireguard_peer_preshared_key(&self, _interface: &str, ros_id: &str) -> Result<Option<String>, String> {
        let path = format!("interface/wireguard/peers/{}", ros_id);
        let res = self.request_json(Method::GET, &path, None).await?;
        Ok(res.get("preshared-key").and_then(|v| v.as_str()).map(|s| s.to_string()))
    }

    pub async fn set_peer_name(&self, _interface: &str, ros_id: &str, name: &str) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "name": name
        });
        self.request_json(Method::POST, "interface/wireguard/peers/set", Some(body)).await?;
        Ok(())
    }

    pub async fn set_peer_client_endpoint(&self, _interface: &str, ros_id: &str, endpoint: Option<&str>) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "client-endpoint": endpoint.unwrap_or("")
        });
        self.request_json(Method::POST, "interface/wireguard/peers/set", Some(body)).await?;
        Ok(())
    }

    pub async fn set_peer_preshared_key(&self, _interface: &str, ros_id: &str, psk: Option<&str>) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "preshared-key": psk.unwrap_or("")
        });
        self.request_json(Method::POST, "interface/wireguard/peers/set", Some(body)).await?;
        Ok(())
    }

    pub async fn get_wireguard_interface(&self, interface: &str) -> Result<WGInterfaceConfig, String> {
        let path = format!("interface/wireguard/{}", interface);
        let res = self.request_json(Method::GET, &path, None).await?;

        let name = res.get("name").and_then(|v| v.as_str()).unwrap_or(interface).to_string();
        let public_key = res.get("public-key").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let listen_port = res.get("listen-port").and_then(|v| v.as_u64()).unwrap_or(13231) as u16;
        let private_key = res.get("private-key").and_then(|v| v.as_str()).map(|s| s.to_string());

        let mut addresses = Vec::new();
        if let Ok(addr_res) = self.request_json(Method::GET, "ip/address", None).await {
            if let Some(arr) = addr_res.as_array() {
                for item in arr {
                    if item.get("interface").and_then(|v| v.as_str()) == Some(interface) {
                        if let Some(a) = item.get("address").and_then(|v| v.as_str()) {
                            addresses.push(a.to_string());
                        }
                    }
                }
            }
        }

        Ok(WGInterfaceConfig {
            name,
            public_key,
            listen_port,
            private_key,
            addresses,
        })
    }

    pub async fn get_primary_ipv4(&self) -> Result<String, String> {
        let res = self.request_json(Method::GET, "ip/address", None).await?;
        if let Some(arr) = res.as_array() {
            for item in arr {
                if let Some(addr) = item.get("address").and_then(|v| v.as_str()) {
                    if let Some(ip) = addr.split('/').next() {
                        if !ip.starts_with("127.") {
                            return Ok(ip.to_string());
                        }
                    }
                }
            }
        }
        Err("No IPv4 address found".to_string())
    }

    pub async fn add_simple_queue(&self, name: &str, target: &str, max_limit_up: &str, max_limit_down: &str, comment: &str) -> Result<String, String> {
        let max_limit = format!("{}/{}", max_limit_up, max_limit_down);
        let body = json!({
            "name": name,
            "target": target,
            "max-limit": max_limit,
            "comment": comment
        });
        let res = self.request_json(Method::PUT, "queue/simple", Some(body)).await?;
        if let Some(ret) = res.get("ret").and_then(|v| v.as_str()) {
            return Ok(ret.to_string());
        }
        if let Some(id) = res.get(".id").and_then(|v| v.as_str()) {
            return Ok(id.to_string());
        }
        Ok(String::new())
    }

    pub async fn update_simple_queue(&self, ros_id: &str, max_limit_up: &str, max_limit_down: &str) -> Result<(), String> {
        let max_limit = format!("{}/{}", max_limit_up, max_limit_down);
        let body = json!({
            "numbers": ros_id,
            "max-limit": max_limit
        });
        self.request_json(Method::POST, "queue/simple/set", Some(body)).await?;
        Ok(())
    }

    pub async fn set_simple_queue_name(&self, ros_id: &str, name: &str) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "name": name
        });
        self.request_json(Method::POST, "queue/simple/set", Some(body)).await?;
        Ok(())
    }

    pub async fn remove_simple_queue(&self, ros_id: &str) -> Result<(), String> {
        let path = format!("queue/simple/{}", ros_id);
        self.request_json(Method::DELETE, &path, None).await?;
        Ok(())
    }

    pub async fn list_simple_queues(&self, name_prefix: &str) -> Result<Vec<SimpleQueueInfo>, String> {
        let res = self.request_json(Method::GET, "queue/simple", None).await?;
        let mut list = Vec::new();
        if let Some(arr) = res.as_array() {
            for item in arr {
                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if name.starts_with(name_prefix) {
                    list.push(SimpleQueueInfo {
                        ros_id: item.get(".id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        name,
                        target: item.get("target").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        max_limit: item.get("max-limit").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        comment: item.get("comment").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    });
                }
            }
        }
        Ok(list)
    }
}
