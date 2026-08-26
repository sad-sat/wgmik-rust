use super::{RouterOSClient, SimpleQueueInfo, WGInterfaceConfig, WGPeer};
use async_trait::async_trait;
use reqwest::{Client, Method};
use serde_json::{json, Value};
use std::net::Ipv4Addr;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct RouterOSRestClient {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub tls_verify: bool,
    pub https: bool,
    pub allow_scheme_fallback: bool,
    pub timeout: Duration,
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
        timeout: Duration,
    ) -> Self {
        Self {
            host,
            port,
            username,
            password,
            tls_verify,
            https,
            allow_scheme_fallback,
            timeout,
        }
    }

    fn base_url(&self, https: bool) -> String {
        let scheme = if https { "https" } else { "http" };
        format!("{}://{}:{}/rest", scheme, self.host, self.port)
    }

    fn build_client(&self, _https: bool) -> Result<Client, String> {
        let mut builder = Client::builder()
            .timeout(self.timeout);
        if !self.tls_verify {
            builder = builder.danger_accept_invalid_certs(true);
        }
        builder.build().map_err(|e| format!("HTTP client error: {}", e))
    }

    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value, String> {
        let client = self.build_client(self.https)?;
        let url = format!("{}{}", self.base_url(self.https), path);

        let mut req = client.request(method.clone(), &url)
            .basic_auth(&self.username, Some(&self.password));
        if let Some(ref b) = body {
            req = req.json(b);
        }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        Ok(v)
                    } else {
                        Ok(Value::String(text))
                    }
                } else {
                    Err(format!("RouterOS HTTP {}: {}", status, text))
                }
            }
            Err(err) => {
                if !self.allow_scheme_fallback {
                    return Err(format!("RouterOS connection failed: {}", err));
                }
                let alt_https = !self.https;
                let alt_client = self.build_client(alt_https)?;
                let alt_url = format!("{}{}", self.base_url(alt_https), path);
                let mut alt_req = alt_client.request(method, &alt_url)
                    .basic_auth(&self.username, Some(&self.password));
                if let Some(ref b) = body {
                    alt_req = alt_req.json(b);
                }
                let resp = alt_req.send().await.map_err(|e| format!("RouterOS fallback failed: {}", e))?;
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        Ok(v)
                    } else {
                        Ok(Value::String(text))
                    }
                } else {
                    Err(format!("RouterOS fallback HTTP {}: {}", status, text))
                }
            }
        }
    }

    fn parse_bool(&self, value: &Value) -> bool {
        match value {
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_i64().map(|v| v != 0).unwrap_or(false),
            Value::String(s) => {
                let lower = s.trim().to_lowercase();
                lower == "true" || lower == "yes" || lower == "1" || lower == "on" || lower == "enabled"
            }
            _ => false,
        }
    }

    fn parse_last_handshake(&self, value: &Value) -> Option<i64> {
        match value {
            Value::Null => None,
            Value::Number(n) => n.as_i64(),
            Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() || trimmed == "none" {
                    return None;
                }
                if let Ok(num) = trimmed.parse::<i64>() {
                    return Some(num);
                }
                let mut total_secs: i64 = 0;
                let mut current_num = String::new();
                for ch in trimmed.chars() {
                    if ch.is_ascii_digit() {
                        current_num.push(ch);
                    } else if !current_num.is_empty() {
                        let val: i64 = current_num.parse().unwrap_or(0);
                        current_num.clear();
                        match ch {
                            'w' => total_secs += val * 604800,
                            'd' => total_secs += val * 86400,
                            'h' => total_secs += val * 3600,
                            'm' => total_secs += val * 60,
                            's' => total_secs += val,
                            _ => {}
                        }
                    }
                }
                if total_secs > 0 {
                    Some(total_secs)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[async_trait]
impl RouterOSClient for RouterOSRestClient {
    async fn get_system_version(&self) -> Result<String, String> {
        let val = self.request(Method::GET, "/system/resource", None).await?;
        if let Some(arr) = val.as_array() {
            if let Some(first) = arr.first() {
                return Ok(first.get("version").and_then(|v| v.as_str()).unwrap_or("").trim().to_string());
            }
        } else if let Some(obj) = val.as_object() {
            return Ok(obj.get("version").and_then(|v| v.as_str()).unwrap_or("").trim().to_string());
        }
        Ok(String::new())
    }

    async fn list_wireguard_interfaces(&self) -> Result<Vec<String>, String> {
        let val = self.request(Method::GET, "/interface/wireguard", None).await?;
        let mut names = Vec::new();
        if let Some(arr) = val.as_array() {
            for row in arr {
                if let Some(name) = row.get("name").and_then(|v| v.as_str()) {
                    if !name.is_empty() {
                        names.push(name.to_string());
                    }
                }
            }
        }
        Ok(names)
    }

    async fn list_all_wireguard_peers(&self) -> Result<Vec<WGPeer>, String> {
        let val = self.request(Method::GET, "/interface/wireguard/peers", None).await?;
        let mut peers = Vec::new();
        if let Some(arr) = val.as_array() {
            for row in arr {
                let ros_id = row.get(".id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let interface = row.get("interface").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let public_key = row.get("public-key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let allowed_address = row.get("allowed-address").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let disabled = row.get("disabled").map(|v| self.parse_bool(v)).unwrap_or(false);
                let rx_bytes = row.get("rx").and_then(|v| {
                    if let Some(n) = v.as_i64() { Some(n) } else if let Some(s) = v.as_str() { s.parse::<i64>().ok() } else { None }
                }).unwrap_or(0);
                let tx_bytes = row.get("tx").and_then(|v| {
                    if let Some(n) = v.as_i64() { Some(n) } else if let Some(s) = v.as_str() { s.parse::<i64>().ok() } else { None }
                }).unwrap_or(0);
                let last_handshake = row.get("last-handshake").and_then(|v| self.parse_last_handshake(v));
                let endpoint = row.get("current-endpoint-address").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let client_endpoint = row.get("client-endpoint")
                    .or_else(|| row.get("clientEndpoint"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();

                peers.push(WGPeer {
                    ros_id,
                    interface,
                    name,
                    public_key,
                    allowed_address,
                    disabled,
                    rx_bytes,
                    tx_bytes,
                    last_handshake,
                    endpoint,
                    client_endpoint,
                });
            }
        }
        Ok(peers)
    }

    async fn list_wireguard_peers(&self, interface: &str) -> Result<Vec<WGPeer>, String> {
        let all = self.list_all_wireguard_peers().await?;
        Ok(all.into_iter().filter(|p| p.interface == interface).collect())
    }

    async fn set_peer_disabled(&self, _interface: &str, ros_id: &str, disabled: bool) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "disabled": if disabled { "yes" } else { "no" }
        });
        let _ = self.request(Method::POST, "/interface/wireguard/peers/set", Some(body)).await?;
        Ok(())
    }

    async fn set_peer_keys(&self, _interface: &str, ros_id: &str, public_key: &str, private_key: &str) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "public-key": public_key,
            "private-key": private_key
        });
        let _ = self.request(Method::POST, "/interface/wireguard/peers/set", Some(body)).await?;
        Ok(())
    }

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
    ) -> Result<String, String> {
        let mut map = serde_json::Map::new();
        map.insert("interface".to_string(), json!(interface));
        map.insert("public-key".to_string(), json!(public_key));
        map.insert("allowed-address".to_string(), json!(allowed_address));
        if !name.is_empty() {
            map.insert("name".to_string(), json!(name));
        }
        if disabled {
            map.insert("disabled".to_string(), json!("yes"));
        }
        if let Some(pk) = private_key {
            if !pk.is_empty() {
                map.insert("private-key".to_string(), json!(pk));
            }
        }
        if let Some(psk) = preshared_key {
            if !psk.is_empty() {
                map.insert("preshared-key".to_string(), json!(psk));
            }
        }
        if let Some(cep) = client_endpoint {
            if !cep.is_empty() {
                map.insert("client-endpoint".to_string(), json!(cep));
            }
        }

        let res = self.request(Method::POST, "/interface/wireguard/peers/add", Some(Value::Object(map))).await?;
        if let Some(rid) = res.get("ret").or_else(|| res.get(".id")).and_then(|v| v.as_str()) {
            if !rid.is_empty() {
                return Ok(rid.to_string());
            }
        }

        // Fallback: locate by pubkey
        let peers = self.list_wireguard_peers(interface).await?;
        for p in peers {
            if p.public_key == public_key {
                return Ok(p.ros_id);
            }
        }
        Err("RouterOS did not return peer id".to_string())
    }

    async fn remove_wireguard_peer(&self, _interface: &str, ros_id: &str) -> Result<(), String> {
        let path = format!("/interface/wireguard/peers/{}", ros_id);
        let _ = self.request(Method::DELETE, &path, None).await?;
        Ok(())
    }

    async fn get_wireguard_peer_private_key(&self, interface: &str, ros_id: &str) -> Result<Option<String>, String> {
        if let Ok(row) = self.request(Method::GET, &format!("/interface/wireguard/peers/{}", ros_id), None).await {
            if let Some(pk) = row.get("private-key").and_then(|v| v.as_str()) {
                let trimmed = pk.trim();
                if !trimmed.is_empty() {
                    return Ok(Some(trimmed.to_string()));
                }
            }
        }
        if let Ok(val) = self.request(Method::GET, "/interface/wireguard/peers", None).await {
            if let Some(arr) = val.as_array() {
                for r in arr {
                    if r.get(".id").and_then(|v| v.as_str()) == Some(ros_id) && r.get("interface").and_then(|v| v.as_str()) == Some(interface) {
                        if let Some(pk) = r.get("private-key").and_then(|v| v.as_str()) {
                            let trimmed = pk.trim();
                            if !trimmed.is_empty() {
                                return Ok(Some(trimmed.to_string()));
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    async fn get_wireguard_peer_preshared_key(&self, interface: &str, ros_id: &str) -> Result<Option<String>, String> {
        if let Ok(row) = self.request(Method::GET, &format!("/interface/wireguard/peers/{}", ros_id), None).await {
            if let Some(pk) = row.get("preshared-key").and_then(|v| v.as_str()) {
                let trimmed = pk.trim();
                if !trimmed.is_empty() {
                    return Ok(Some(trimmed.to_string()));
                }
            }
        }
        if let Ok(val) = self.request(Method::GET, "/interface/wireguard/peers", None).await {
            if let Some(arr) = val.as_array() {
                for r in arr {
                    if r.get(".id").and_then(|v| v.as_str()) == Some(ros_id) && r.get("interface").and_then(|v| v.as_str()) == Some(interface) {
                        if let Some(pk) = r.get("preshared-key").and_then(|v| v.as_str()) {
                            let trimmed = pk.trim();
                            if !trimmed.is_empty() {
                                return Ok(Some(trimmed.to_string()));
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    async fn set_peer_name(&self, _interface: &str, ros_id: &str, name: &str) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "name": name
        });
        let _ = self.request(Method::POST, "/interface/wireguard/peers/set", Some(body)).await?;
        Ok(())
    }

    async fn set_peer_client_endpoint(&self, _interface: &str, ros_id: &str, client_endpoint: Option<&str>) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "client-endpoint": client_endpoint.unwrap_or("").trim()
        });
        let _ = self.request(Method::POST, "/interface/wireguard/peers/set", Some(body)).await?;
        Ok(())
    }

    async fn set_peer_preshared_key(&self, _interface: &str, ros_id: &str, preshared_key: Option<&str>) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "preshared-key": preshared_key.unwrap_or("")
        });
        let _ = self.request(Method::POST, "/interface/wireguard/peers/set", Some(body)).await?;
        Ok(())
    }

    async fn get_wireguard_interface(&self, interface: &str) -> Result<WGInterfaceConfig, String> {
        let val = self.request(Method::GET, "/interface/wireguard", None).await?;
        if let Some(arr) = val.as_array() {
            for row in arr {
                if row.get("name").and_then(|v| v.as_str()) == Some(interface) {
                    let public_key = row.get("public-key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let listen_port = row.get("listen-port").and_then(|v| {
                        if let Some(n) = v.as_u64() { Some(n as u16) } else if let Some(s) = v.as_str() { s.parse::<u16>().ok() } else { None }
                    }).unwrap_or(0);

                    // Fetch interface addresses from /ip/address
                    let mut addresses = Vec::new();
                    if let Ok(addr_val) = self.request(Method::GET, "/ip/address", None).await {
                        if let Some(addr_arr) = addr_val.as_array() {
                            for addr_row in addr_arr {
                                if addr_row.get("interface").and_then(|v| v.as_str()) == Some(interface) {
                                    let disabled = addr_row.get("disabled").map(|v| self.parse_bool(v)).unwrap_or(false);
                                    if !disabled {
                                        if let Some(a) = addr_row.get("address").and_then(|v| v.as_str()) {
                                            if !a.trim().is_empty() {
                                                addresses.push(a.trim().to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    return Ok(WGInterfaceConfig {
                        name: interface.to_string(),
                        public_key,
                        listen_port,
                        addresses,
                    });
                }
            }
        }
        Err(format!("WireGuard interface '{}' not found", interface))
    }

    async fn get_primary_ipv4(&self) -> Result<String, String> {
        let val = self.request(Method::GET, "/ip/address", None).await.unwrap_or(Value::Array(Vec::new()));
        let mut public: Option<String> = None;
        let mut private: Option<String> = None;

        if let Some(arr) = val.as_array() {
            for row in arr {
                if let Some(addr_str) = row.get("address").and_then(|v| v.as_str()) {
                    let ip_part = addr_str.split('/').next().unwrap_or("");
                    if let Ok(ip) = ip_part.parse::<Ipv4Addr>() {
                        if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() {
                            if ip.is_private() {
                                if private.is_none() {
                                    private = Some(ip.to_string());
                                }
                            } else if public.is_none() {
                                public = Some(ip.to_string());
                            }
                        }
                    }
                }
            }
        }
        Ok(public.or(private).unwrap_or_default())
    }

    async fn add_simple_queue(&self, name: &str, target: &str, max_limit_up: &str, max_limit_down: &str, comment: &str) -> Result<String, String> {
        let mut map = serde_json::Map::new();
        map.insert("name".to_string(), json!(name));
        map.insert("target".to_string(), json!(target));
        map.insert("max-limit".to_string(), json!(format!("{}/{}", max_limit_up, max_limit_down)));
        if !comment.is_empty() {
            map.insert("comment".to_string(), json!(comment));
        }

        let res = self.request(Method::POST, "/queue/simple/add", Some(Value::Object(map))).await?;
        if let Some(rid) = res.get("ret").or_else(|| res.get(".id")).and_then(|v| v.as_str()) {
            if !rid.is_empty() {
                return Ok(rid.to_string());
            }
        }

        let queues = self.list_simple_queues("").await?;
        for q in queues {
            if q.name == name {
                return Ok(q.ros_id);
            }
        }
        Err("RouterOS did not return queue id".to_string())
    }

    async fn update_simple_queue(&self, ros_id: &str, max_limit_up: &str, max_limit_down: &str) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "max-limit": format!("{}/{}", max_limit_up, max_limit_down)
        });
        let _ = self.request(Method::POST, "/queue/simple/set", Some(body)).await?;
        Ok(())
    }

    async fn set_simple_queue_name(&self, ros_id: &str, name: &str) -> Result<(), String> {
        let body = json!({
            "numbers": ros_id,
            "name": name
        });
        let _ = self.request(Method::POST, "/queue/simple/set", Some(body)).await?;
        Ok(())
    }

    async fn remove_simple_queue(&self, ros_id: &str) -> Result<(), String> {
        let path = format!("/queue/simple/{}", ros_id);
        let _ = self.request(Method::DELETE, &path, None).await?;
        Ok(())
    }

    async fn list_simple_queues(&self, name_prefix: &str) -> Result<Vec<SimpleQueueInfo>, String> {
        let val = self.request(Method::GET, "/queue/simple", None).await?;
        let mut queues = Vec::new();
        if let Some(arr) = val.as_array() {
            for row in arr {
                let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !name_prefix.is_empty() && !name.starts_with(name_prefix) {
                    continue;
                }
                let ros_id = row.get(".id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let target = row.get("target").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let max_limit = row.get("max-limit").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let comment = row.get("comment").and_then(|v| v.as_str()).unwrap_or("").to_string();

                queues.push(SimpleQueueInfo {
                    ros_id,
                    name,
                    target,
                    max_limit,
                    comment,
                });
            }
        }
        Ok(queues)
    }
}
