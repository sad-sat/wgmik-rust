use super::{SimpleQueueInfo, WGInterfaceConfig, WGPeer};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

pub struct RouterOSApiClient {
    host: String,
    port: u16,
    username: String,
    password: String,
    use_tls: bool,
    ssl_verify: bool,
    timeout_duration: Duration,
}

impl RouterOSApiClient {
    pub fn new(
        host: String,
        port: u16,
        username: String,
        password: String,
        use_tls: bool,
        ssl_verify: bool,
        timeout_duration: Duration,
    ) -> Self {
        Self {
            host,
            port,
            username,
            password,
            use_tls,
            ssl_verify,
            timeout_duration,
        }
    }

    async fn encode_word(w: &str) -> Vec<u8> {
        let b = w.as_bytes();
        let l = b.len();
        let mut out = Vec::new();
        if l < 0x80 {
            out.push(l as u8);
        } else if l < 0x4000 {
            let val = l | 0x8000;
            out.push((val >> 8) as u8);
            out.push((val & 0xFF) as u8);
        } else if l < 0x200000 {
            let val = l | 0xC00000;
            out.push((val >> 16) as u8);
            out.push(((val >> 8) & 0xFF) as u8);
            out.push((val & 0xFF) as u8);
        } else if l < 0x10000000 {
            let val = l | 0xE0000000;
            out.push((val >> 24) as u8);
            out.push(((val >> 16) & 0xFF) as u8);
            out.push(((val >> 8) & 0xFF) as u8);
            out.push((val & 0xFF) as u8);
        } else {
            out.push(0xF0);
            out.push((l >> 24) as u8);
            out.push(((l >> 16) & 0xFF) as u8);
            out.push(((l >> 8) & 0xFF) as u8);
            out.push((l & 0xFF) as u8);
        }
        out.extend_from_slice(b);
        out
    }

    async fn read_word<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<u8>, String> {
        let b = reader.read_u8().await.map_err(|e| e.to_string())?;
        let len = if b & 0x80 == 0 {
            b as usize
        } else if b & 0xC0 == 0x80 {
            let b2 = reader.read_u8().await.map_err(|e| e.to_string())?;
            (((b as usize) & 0x3F) << 8) | (b2 as usize)
        } else if b & 0xE0 == 0xC0 {
            let b2 = reader.read_u8().await.map_err(|e| e.to_string())?;
            let b3 = reader.read_u8().await.map_err(|e| e.to_string())?;
            (((b as usize) & 0x1F) << 16) | ((b2 as usize) << 8) | (b3 as usize)
        } else if b & 0xF0 == 0xE0 {
            let b2 = reader.read_u8().await.map_err(|e| e.to_string())?;
            let b3 = reader.read_u8().await.map_err(|e| e.to_string())?;
            let b4 = reader.read_u8().await.map_err(|e| e.to_string())?;
            (((b as usize) & 0x0F) << 24) | ((b2 as usize) << 16) | ((b3 as usize) << 8) | (b4 as usize)
        } else if b == 0xF0 {
            let b1 = reader.read_u8().await.map_err(|e| e.to_string())?;
            let b2 = reader.read_u8().await.map_err(|e| e.to_string())?;
            let b3 = reader.read_u8().await.map_err(|e| e.to_string())?;
            let b4 = reader.read_u8().await.map_err(|e| e.to_string())?;
            ((b1 as usize) << 24) | ((b2 as usize) << 16) | ((b3 as usize) << 8) | (b4 as usize)
        } else {
            return Err("Unknown length prefix in API stream".to_string());
        };

        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
        Ok(buf)
    }

    pub async fn execute_sentence(&self, words: &[String]) -> Result<Vec<HashMap<String, String>>, String> {
        let addr = format!("{}:{}", self.host, self.port);
        let stream = timeout(self.timeout_duration, TcpStream::connect(&addr))
            .await
            .map_err(|_| "Connection timeout".to_string())?
            .map_err(|e| format!("TCP connect failed: {}", e))?;

        let (mut reader, mut writer) = tokio::io::split(stream);

        // Login command
        let mut login_payload = Vec::new();
        login_payload.extend(Self::encode_word("/login").await);
        login_payload.extend(Self::encode_word(&format!("=name={}", self.username)).await);
        login_payload.extend(Self::encode_word(&format!("=password={}", self.password)).await);
        login_payload.push(0);

        writer.write_all(&login_payload).await.map_err(|e| e.to_string())?;

        // Read login response
        loop {
            let word_bytes = Self::read_word(&mut reader).await?;
            if word_bytes.is_empty() {
                break;
            }
            let word = String::from_utf8_lossy(&word_bytes);
            if word == "!done" {
                break;
            } else if word.starts_with("!trap") {
                return Err("RouterOS login failed: bad credentials".to_string());
            }
        }

        // Send actual command
        let mut cmd_payload = Vec::new();
        for w in words {
            cmd_payload.extend(Self::encode_word(w).await);
        }
        cmd_payload.push(0);
        writer.write_all(&cmd_payload).await.map_err(|e| e.to_string())?;

        // Read response rows
        let mut results = Vec::new();
        let mut current_row: HashMap<String, String> = HashMap::new();

        loop {
            let word_bytes = Self::read_word(&mut reader).await?;
            if word_bytes.is_empty() {
                if !current_row.is_empty() {
                    results.push(current_row);
                    current_row = HashMap::new();
                }
                continue;
            }
            let word = String::from_utf8_lossy(&word_bytes);
            if word == "!done" {
                if !current_row.is_empty() {
                    results.push(current_row);
                }
                break;
            } else if word == "!re" {
                if !current_row.is_empty() {
                    results.push(current_row);
                    current_row = HashMap::new();
                }
            } else if word.starts_with("!trap") {
                return Err(format!("RouterOS error: {}", word));
            } else if word.starts_with('=') {
                if let Some(eq_idx) = word[1..].find('=') {
                    let k = &word[1..=eq_idx];
                    let v = &word[eq_idx + 2..];
                    current_row.insert(k.to_string(), v.to_string());
                }
            }
        }

        Ok(results)
    }

    pub async fn get_system_version(&self) -> Result<String, String> {
        let rows = self.execute_sentence(&["/system/resource/print".to_string()]).await?;
        if let Some(first) = rows.first() {
            if let Some(v) = first.get("version") {
                return Ok(v.clone());
            }
        }
        Err("Could not determine RouterOS version".to_string())
    }

    pub async fn list_wireguard_interfaces(&self) -> Result<Vec<String>, String> {
        let rows = self.execute_sentence(&["/interface/wireguard/print".to_string()]).await?;
        let mut list = Vec::new();
        for r in rows {
            if let Some(name) = r.get("name") {
                list.push(name.clone());
            }
        }
        Ok(list)
    }

    pub async fn list_all_wireguard_peers(&self) -> Result<Vec<WGPeer>, String> {
        let rows = self.execute_sentence(&["/interface/wireguard/peers/print".to_string()]).await?;
        let mut peers = Vec::new();
        for row in rows {
            let ros_id = row.get(".id").cloned().unwrap_or_default();
            let interface = row.get("interface").cloned().unwrap_or_default();
            let public_key = row.get("public-key").cloned().unwrap_or_default();
            let allowed_address = row.get("allowed-address").cloned().unwrap_or_default();
            let disabled = row.get("disabled").map(|v| v == "true" || v == "yes").unwrap_or(false);
            let rx_bytes = row.get("rx").and_then(|v| v.parse().ok()).unwrap_or(0);
            let tx_bytes = row.get("tx").and_then(|v| v.parse().ok()).unwrap_or(0);
            let name = row.get("name").cloned().unwrap_or_default();
            let current_endpoint_address = row.get("current-endpoint-address").cloned();
            let endpoint = current_endpoint_address.clone().or_else(|| row.get("endpoint-address").cloned());
            let client_endpoint = row.get("client-endpoint").cloned().unwrap_or_default().trim().to_string();
            let last_handshake = row.get("last-handshake").and_then(|v| super::parse_last_handshake_str(v));

            peers.push(WGPeer {
                ros_id,
                interface,
                name,
                public_key,
                allowed_address,
                endpoint,
                current_endpoint_address,
                rx_bytes,
                tx_bytes,
                last_handshake,
                disabled,
                comment: None,
                client_endpoint: if client_endpoint.is_empty() { None } else { Some(client_endpoint) },
                client_listen_port: None,
            });
        }
        Ok(peers)
    }

    pub async fn list_wireguard_peers(&self, interface: &str) -> Result<Vec<WGPeer>, String> {
        let all = self.list_all_wireguard_peers().await?;
        Ok(all.into_iter().filter(|p| p.interface == interface).collect())
    }

    pub async fn set_peer_disabled(&self, _interface: &str, ros_id: &str, disabled: bool) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=disabled={}", if disabled { "yes" } else { "no" }),
        ]).await?;
        Ok(())
    }

    pub async fn set_peer_keys(&self, _interface: &str, ros_id: &str, public_key: &str, private_key: &str) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=public-key={}", public_key),
            format!("=private-key={}", private_key),
        ]).await?;
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
        let mut words = vec![
            "/interface/wireguard/peers/add".to_string(),
            format!("=interface={}", interface),
            format!("=public-key={}", public_key),
            format!("=allowed-address={}", allowed_address),
            format!("=name={}", name),
            format!("=disabled={}", if disabled { "yes" } else { "no" }),
        ];
        if let Some(priv_k) = private_key {
            if !priv_k.is_empty() {
                words.push(format!("=private-key={}", priv_k));
            }
        }
        if let Some(psk) = preshared_key {
            if !psk.is_empty() {
                words.push(format!("=preshared-key={}", psk));
            }
        }
        if let Some(ep) = client_endpoint {
            if !ep.is_empty() {
                words.push(format!("=client-endpoint={}", ep));
            }
        }

        let res = self.execute_sentence(&words).await?;
        if let Some(first) = res.first() {
            if let Some(ret) = first.get("ret").or_else(|| first.get(".id")) {
                return Ok(ret.clone());
            }
        }
        Ok(String::new())
    }

    pub async fn remove_wireguard_peer(&self, _interface: &str, ros_id: &str) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/remove".to_string(),
            format!("=.id={}", ros_id),
        ]).await?;
        Ok(())
    }

    pub async fn get_wireguard_peer_private_key(&self, _interface: &str, ros_id: &str) -> Result<Option<String>, String> {
        let rows = self.execute_sentence(&[
            "/interface/wireguard/peers/print".to_string(),
            format!("?.id={}", ros_id),
        ]).await?;
        Ok(rows.first().and_then(|r| r.get("private-key").cloned()))
    }

    pub async fn get_wireguard_peer_preshared_key(&self, _interface: &str, ros_id: &str) -> Result<Option<String>, String> {
        let rows = self.execute_sentence(&[
            "/interface/wireguard/peers/print".to_string(),
            format!("?.id={}", ros_id),
        ]).await?;
        Ok(rows.first().and_then(|r| r.get("preshared-key").cloned()))
    }

    pub async fn set_peer_name(&self, _interface: &str, ros_id: &str, name: &str) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=name={}", name),
        ]).await?;
        Ok(())
    }

    pub async fn set_peer_client_endpoint(&self, _interface: &str, ros_id: &str, endpoint: Option<&str>) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=client-endpoint={}", endpoint.unwrap_or("")),
        ]).await?;
        Ok(())
    }

    pub async fn set_peer_preshared_key(&self, _interface: &str, ros_id: &str, psk: Option<&str>) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=preshared-key={}", psk.unwrap_or("")),
        ]).await?;
        Ok(())
    }

    pub async fn get_wireguard_interface(&self, interface: &str) -> Result<WGInterfaceConfig, String> {
        let rows = self.execute_sentence(&[
            "/interface/wireguard/print".to_string(),
            format!("?name={}", interface),
        ]).await?;
        let first = rows.first().ok_or_else(|| "Interface not found".to_string())?;

        let name = first.get("name").cloned().unwrap_or_else(|| interface.to_string());
        let public_key = first.get("public-key").cloned().unwrap_or_default();
        let listen_port = first.get("listen-port").and_then(|v| v.parse().ok()).unwrap_or(13231);
        let private_key = first.get("private-key").cloned();

        let mut addresses = Vec::new();
        if let Ok(addr_rows) = self.execute_sentence(&["/ip/address/print".to_string(), format!("?interface={}", interface)]).await {
            for ar in addr_rows {
                if let Some(a) = ar.get("address") {
                    addresses.push(a.clone());
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
        let rows = self.execute_sentence(&["/ip/address/print".to_string()]).await?;
        for r in rows {
            if let Some(addr) = r.get("address") {
                if let Some(ip) = addr.split('/').next() {
                    if !ip.starts_with("127.") {
                        return Ok(ip.to_string());
                    }
                }
            }
        }
        Err("No IPv4 address found".to_string())
    }

    pub async fn add_simple_queue(&self, name: &str, target: &str, max_limit_up: &str, max_limit_down: &str, comment: &str) -> Result<String, String> {
        let max_limit = format!("{}/{}", max_limit_up, max_limit_down);
        let rows = self.execute_sentence(&[
            "/queue/simple/add".to_string(),
            format!("=name={}", name),
            format!("=target={}", target),
            format!("=max-limit={}", max_limit),
            format!("=comment={}", comment),
        ]).await?;

        if let Some(first) = rows.first() {
            if let Some(ret) = first.get("ret").or_else(|| first.get(".id")) {
                return Ok(ret.clone());
            }
        }
        Ok(String::new())
    }

    pub async fn update_simple_queue(&self, ros_id: &str, max_limit_up: &str, max_limit_down: &str) -> Result<(), String> {
        let max_limit = format!("{}/{}", max_limit_up, max_limit_down);
        self.execute_sentence(&[
            "/queue/simple/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=max-limit={}", max_limit),
        ]).await?;
        Ok(())
    }

    pub async fn set_simple_queue_name(&self, ros_id: &str, name: &str) -> Result<(), String> {
        self.execute_sentence(&[
            "/queue/simple/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=name={}", name),
        ]).await?;
        Ok(())
    }

    pub async fn remove_simple_queue(&self, ros_id: &str) -> Result<(), String> {
        self.execute_sentence(&[
            "/queue/simple/remove".to_string(),
            format!("=.id={}", ros_id),
        ]).await?;
        Ok(())
    }

    pub async fn list_simple_queues(&self, name_prefix: &str) -> Result<Vec<SimpleQueueInfo>, String> {
        let rows = self.execute_sentence(&["/queue/simple/print".to_string()]).await?;
        let mut list = Vec::new();
        for r in rows {
            let name = r.get("name").cloned().unwrap_or_default();
            if name.starts_with(name_prefix) {
                list.push(SimpleQueueInfo {
                    ros_id: r.get(".id").cloned().unwrap_or_default(),
                    name,
                    target: r.get("target").cloned().unwrap_or_default(),
                    max_limit: r.get("max-limit").cloned().unwrap_or_default(),
                    comment: r.get("comment").cloned(),
                });
            }
        }
        Ok(list)
    }
}
