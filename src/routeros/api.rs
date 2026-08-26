use super::{RouterOSClient, SimpleQueueInfo, WGInterfaceConfig, WGPeer};
use async_trait::async_trait;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Clone, Debug)]
pub struct RouterOSApiClient {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub use_tls: bool,
    pub ssl_verify: bool,
    pub timeout: Duration,
}

impl RouterOSApiClient {
    pub fn new(
        host: String,
        port: u16,
        username: String,
        password: String,
        use_tls: bool,
        ssl_verify: bool,
        timeout: Duration,
    ) -> Self {
        Self {
            host,
            port,
            username,
            password,
            use_tls,
            ssl_verify,
            timeout,
        }
    }

    async fn encode_word<W: AsyncWriteExt + Unpin>(writer: &mut W, word: &[u8]) -> Result<(), String> {
        let len = word.len();
        if len < 0x80 {
            writer.write_all(&[len as u8]).await.map_err(|e| e.to_string())?;
        } else if len < 0x4000 {
            let b1 = 0x80 | ((len >> 8) as u8);
            let b2 = (len & 0xFF) as u8;
            writer.write_all(&[b1, b2]).await.map_err(|e| e.to_string())?;
        } else if len < 0x200000 {
            let b1 = 0xC0 | ((len >> 16) as u8);
            let b2 = ((len >> 8) & 0xFF) as u8;
            let b3 = (len & 0xFF) as u8;
            writer.write_all(&[b1, b2, b3]).await.map_err(|e| e.to_string())?;
        } else if len < 0x10000000 {
            let b1 = 0xE0 | ((len >> 24) as u8);
            let b2 = ((len >> 16) & 0xFF) as u8;
            let b3 = ((len >> 8) & 0xFF) as u8;
            let b4 = (len & 0xFF) as u8;
            writer.write_all(&[b1, b2, b3, b4]).await.map_err(|e| e.to_string())?;
        } else {
            let b0 = 0xF0;
            let b1 = ((len >> 24) & 0xFF) as u8;
            let b2 = ((len >> 16) & 0xFF) as u8;
            let b3 = ((len >> 8) & 0xFF) as u8;
            let b4 = (len & 0xFF) as u8;
            writer.write_all(&[b0, b1, b2, b3, b4]).await.map_err(|e| e.to_string())?;
        }
        writer.write_all(word).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn decode_word<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Option<Vec<u8>>, String> {
        let mut b = [0u8; 1];
        if reader.read_exact(&mut b).await.is_err() {
            return Ok(None);
        }
        let b1 = b[0];
        let len: usize = if b1 & 0x80 == 0 {
            b1 as usize
        } else if (b1 & 0xC0) == 0x80 {
            let mut b2 = [0u8; 1];
            reader.read_exact(&mut b2).await.map_err(|e| e.to_string())?;
            (((b1 & 0x3F) as usize) << 8) | (b2[0] as usize)
        } else if (b1 & 0xE0) == 0xC0 {
            let mut buf = [0u8; 2];
            reader.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
            (((b1 & 0x1F) as usize) << 16) | ((buf[0] as usize) << 8) | (buf[1] as usize)
        } else if (b1 & 0xF0) == 0xE0 {
            let mut buf = [0u8; 3];
            reader.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
            (((b1 & 0x0F) as usize) << 24) | ((buf[0] as usize) << 16) | ((buf[1] as usize) << 8) | (buf[2] as usize)
        } else if b1 == 0xF0 {
            let mut buf = [0u8; 4];
            reader.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
            ((buf[0] as usize) << 24) | ((buf[1] as usize) << 16) | ((buf[2] as usize) << 8) | (buf[3] as usize)
        } else {
            return Err("Invalid word length encoding".to_string());
        };

        if len == 0 {
            return Ok(Some(Vec::new())); // Empty word terminates sentence
        }

        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
        Ok(Some(buf))
    }

    async fn execute_sentence(&self, words: &[String]) -> Result<Vec<HashMap<String, String>>, String> {
        let addr = format!("{}:{}", self.host, self.port);
        let mut stream = tokio::time::timeout(self.timeout, TcpStream::connect(&addr))
            .await
            .map_err(|_| "Connection timeout".to_string())?
            .map_err(|e| format!("TCP connect failed: {}", e))?;

        // 1. Send Login
        Self::encode_word(&mut stream, b"/login").await?;
        Self::encode_word(&mut stream, format!("=name={}", self.username).as_bytes()).await?;
        Self::encode_word(&mut stream, format!("=password={}", self.password).as_bytes()).await?;
        stream.write_all(&[0x00]).await.map_err(|e| e.to_string())?;

        // Read login reply
        let mut login_ok = false;
        loop {
            let mut reply_sentence = Vec::new();
            loop {
                match Self::decode_word(&mut stream).await? {
                    Some(w) if w.is_empty() => break,
                    Some(w) => reply_sentence.push(String::from_utf8_lossy(&w).to_string()),
                    None => return Err("Unexpected EOF during login".to_string()),
                }
            }
            if let Some(first) = reply_sentence.first() {
                if first == "!done" {
                    login_ok = true;
                    break;
                } else if first == "!trap" {
                    return Err(format!("RouterOS API login failed: {:?}", reply_sentence));
                }
            }
        }

        if !login_ok {
            return Err("RouterOS API login failed".to_string());
        }

        // 2. Send Command Sentence
        for word in words {
            Self::encode_word(&mut stream, word.as_bytes()).await?;
        }
        stream.write_all(&[0x00]).await.map_err(|e| e.to_string())?;

        // 3. Read Reply Sentences
        let mut results = Vec::new();
        loop {
            let mut reply_sentence = Vec::new();
            loop {
                match Self::decode_word(&mut stream).await? {
                    Some(w) if w.is_empty() => break,
                    Some(w) => reply_sentence.push(String::from_utf8_lossy(&w).to_string()),
                    None => return Err("Unexpected EOF during command".to_string()),
                }
            }
            if let Some(first) = reply_sentence.first() {
                if first == "!re" {
                    let mut map = HashMap::new();
                    for attr in &reply_sentence[1..] {
                        if let Some(stripped) = attr.strip_prefix('=') {
                            if let Some((k, v)) = stripped.split_once('=') {
                                map.insert(k.to_string(), v.to_string());
                            }
                        }
                    }
                    results.push(map);
                } else if first == "!done" {
                    let mut map = HashMap::new();
                    for attr in &reply_sentence[1..] {
                        if let Some(stripped) = attr.strip_prefix('=') {
                            if let Some((k, v)) = stripped.split_once('=') {
                                map.insert(k.to_string(), v.to_string());
                            }
                        }
                    }
                    if !map.is_empty() {
                        results.push(map);
                    }
                    break;
                } else if first == "!trap" {
                    return Err(format!("RouterOS command error: {:?}", reply_sentence));
                }
            }
        }

        Ok(results)
    }

    fn parse_bool(&self, value: Option<&String>) -> bool {
        match value.map(|s| s.trim().to_lowercase()) {
            Some(s) if s == "true" || s == "yes" || s == "1" || s == "on" => true,
            _ => false,
        }
    }

    fn parse_last_handshake(&self, value: Option<&String>) -> Option<i64> {
        let s = value?.trim();
        if s.is_empty() || s == "none" {
            return None;
        }
        if let Ok(num) = s.parse::<i64>() {
            return Some(num);
        }
        let mut total_secs: i64 = 0;
        let mut current_num = String::new();
        for ch in s.chars() {
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
}

#[async_trait]
impl RouterOSClient for RouterOSApiClient {
    async fn get_system_version(&self) -> Result<String, String> {
        let rows = self.execute_sentence(&["/system/resource/print".to_string()]).await?;
        if let Some(first) = rows.first() {
            if let Some(ver) = first.get("version") {
                return Ok(ver.trim().to_string());
            }
        }
        Ok(String::new())
    }

    async fn list_wireguard_interfaces(&self) -> Result<Vec<String>, String> {
        let rows = self.execute_sentence(&["/interface/wireguard/print".to_string()]).await?;
        let names = rows.into_iter().filter_map(|r| r.get("name").cloned()).collect();
        Ok(names)
    }

    async fn list_all_wireguard_peers(&self) -> Result<Vec<WGPeer>, String> {
        let rows = self.execute_sentence(&["/interface/wireguard/peers/print".to_string()]).await?;
        let mut peers = Vec::new();
        for row in rows {
            let ros_id = row.get(".id").cloned().unwrap_or_default();
            let interface = row.get("interface").cloned().unwrap_or_default();
            let name = row.get("name").cloned().unwrap_or_default();
            let public_key = row.get("public-key").cloned().unwrap_or_default();
            let allowed_address = row.get("allowed-address").cloned().unwrap_or_default();
            let disabled = self.parse_bool(row.get("disabled"));
            let rx_bytes = row.get("rx").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
            let tx_bytes = row.get("tx").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
            let last_handshake = self.parse_last_handshake(row.get("last-handshake"));
            let endpoint = row.get("current-endpoint-address").cloned().unwrap_or_default();
            let client_endpoint = row.get("client-endpoint").cloned().unwrap_or_default().trim().to_string();

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
        Ok(peers)
    }

    async fn list_wireguard_peers(&self, interface: &str) -> Result<Vec<WGPeer>, String> {
        let all = self.list_all_wireguard_peers().await?;
        Ok(all.into_iter().filter(|p| p.interface == interface).collect())
    }

    async fn set_peer_disabled(&self, _interface: &str, ros_id: &str, disabled: bool) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=disabled={}", if disabled { "yes" } else { "no" }),
        ]).await?;
        Ok(())
    }

    async fn set_peer_keys(&self, _interface: &str, ros_id: &str, public_key: &str, private_key: &str) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=public-key={}", public_key),
            format!("=private-key={}", private_key),
        ]).await?;
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
        let mut words = vec![
            "/interface/wireguard/peers/add".to_string(),
            format!("=interface={}", interface),
            format!("=public-key={}", public_key),
            format!("=allowed-address={}", allowed_address),
        ];
        if !name.is_empty() {
            words.push(format!("=name={}", name));
        }
        if disabled {
            words.push("=disabled=yes".to_string());
        }
        if let Some(pk) = private_key {
            if !pk.is_empty() {
                words.push(format!("=private-key={}", pk));
            }
        }
        if let Some(psk) = preshared_key {
            if !psk.is_empty() {
                words.push(format!("=preshared-key={}", psk));
            }
        }
        if let Some(cep) = client_endpoint {
            if !cep.is_empty() {
                words.push(format!("=client-endpoint={}", cep));
            }
        }

        let res = self.execute_sentence(&words).await?;
        if let Some(first) = res.first() {
            if let Some(rid) = first.get("ret").or_else(|| first.get(".id")) {
                if !rid.is_empty() {
                    return Ok(rid.clone());
                }
            }
        }

        let peers = self.list_wireguard_peers(interface).await?;
        for p in peers {
            if p.public_key == public_key {
                return Ok(p.ros_id);
            }
        }
        Err("RouterOS did not return peer id".to_string())
    }

    async fn remove_wireguard_peer(&self, _interface: &str, ros_id: &str) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/remove".to_string(),
            format!("=.id={}", ros_id),
        ]).await?;
        Ok(())
    }

    async fn get_wireguard_peer_private_key(&self, interface: &str, ros_id: &str) -> Result<Option<String>, String> {
        let rows = self.execute_sentence(&[
            "/interface/wireguard/peers/print".to_string(),
            "=.proplist=.id,interface,private-key".to_string(),
        ]).await.unwrap_or_default();
        for r in rows {
            if r.get(".id").map(|s| s.as_str()) == Some(ros_id) && r.get("interface").map(|s| s.as_str()) == Some(interface) {
                if let Some(pk) = r.get("private-key") {
                    let trimmed = pk.trim();
                    if !trimmed.is_empty() {
                        return Ok(Some(trimmed.to_string()));
                    }
                }
            }
        }
        Ok(None)
    }

    async fn get_wireguard_peer_preshared_key(&self, interface: &str, ros_id: &str) -> Result<Option<String>, String> {
        let rows = self.execute_sentence(&[
            "/interface/wireguard/peers/print".to_string(),
            "=.proplist=.id,interface,preshared-key".to_string(),
        ]).await.unwrap_or_default();
        for r in rows {
            if r.get(".id").map(|s| s.as_str()) == Some(ros_id) && r.get("interface").map(|s| s.as_str()) == Some(interface) {
                if let Some(pk) = r.get("preshared-key") {
                    let trimmed = pk.trim();
                    if !trimmed.is_empty() {
                        return Ok(Some(trimmed.to_string()));
                    }
                }
            }
        }
        Ok(None)
    }

    async fn set_peer_name(&self, _interface: &str, ros_id: &str, name: &str) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=name={}", name),
        ]).await?;
        Ok(())
    }

    async fn set_peer_client_endpoint(&self, _interface: &str, ros_id: &str, client_endpoint: Option<&str>) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=client-endpoint={}", client_endpoint.unwrap_or("").trim()),
        ]).await?;
        Ok(())
    }

    async fn set_peer_preshared_key(&self, _interface: &str, ros_id: &str, preshared_key: Option<&str>) -> Result<(), String> {
        self.execute_sentence(&[
            "/interface/wireguard/peers/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=preshared-key={}", preshared_key.unwrap_or("")),
        ]).await?;
        Ok(())
    }

    async fn get_wireguard_interface(&self, interface: &str) -> Result<WGInterfaceConfig, String> {
        let rows = self.execute_sentence(&["/interface/wireguard/print".to_string()]).await?;
        for row in rows {
            if row.get("name").map(|s| s.as_str()) == Some(interface) {
                let public_key = row.get("public-key").cloned().unwrap_or_default();
                let listen_port = row.get("listen-port").and_then(|v| v.parse::<u16>().ok()).unwrap_or(0);

                let mut addresses = Vec::new();
                if let Ok(addr_rows) = self.execute_sentence(&["/ip/address/print".to_string()]).await {
                    for addr_row in addr_rows {
                        if addr_row.get("interface").map(|s| s.as_str()) == Some(interface) {
                            if !self.parse_bool(addr_row.get("disabled")) {
                                if let Some(a) = addr_row.get("address") {
                                    if !a.trim().is_empty() {
                                        addresses.push(a.trim().to_string());
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
        Err(format!("wireguard interface '{}' not found", interface))
    }

    async fn get_primary_ipv4(&self) -> Result<String, String> {
        let rows = self.execute_sentence(&["/ip/address/print".to_string()]).await.unwrap_or_default();
        let mut public: Option<String> = None;
        let mut private: Option<String> = None;

        for row in rows {
            if let Some(addr_str) = row.get("address") {
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
        Ok(public.or(private).unwrap_or_default())
    }

    async fn add_simple_queue(&self, name: &str, target: &str, max_limit_up: &str, max_limit_down: &str, comment: &str) -> Result<String, String> {
        let mut words = vec![
            "/queue/simple/add".to_string(),
            format!("=name={}", name),
            format!("=target={}", target),
            format!("=max-limit={}/{}", max_limit_up, max_limit_down),
        ];
        if !comment.is_empty() {
            words.push(format!("=comment={}", comment));
        }

        let res = self.execute_sentence(&words).await?;
        if let Some(first) = res.first() {
            if let Some(rid) = first.get("ret").or_else(|| first.get(".id")) {
                if !rid.is_empty() {
                    return Ok(rid.clone());
                }
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
        self.execute_sentence(&[
            "/queue/simple/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=max-limit={}/{}", max_limit_up, max_limit_down),
        ]).await?;
        Ok(())
    }

    async fn set_simple_queue_name(&self, ros_id: &str, name: &str) -> Result<(), String> {
        self.execute_sentence(&[
            "/queue/simple/set".to_string(),
            format!("=.id={}", ros_id),
            format!("=name={}", name),
        ]).await?;
        Ok(())
    }

    async fn remove_simple_queue(&self, ros_id: &str) -> Result<(), String> {
        self.execute_sentence(&[
            "/queue/simple/remove".to_string(),
            format!("=.id={}", ros_id),
        ]).await?;
        Ok(())
    }

    async fn list_simple_queues(&self, name_prefix: &str) -> Result<Vec<SimpleQueueInfo>, String> {
        let rows = self.execute_sentence(&["/queue/simple/print".to_string()]).await?;
        let mut queues = Vec::new();
        for row in rows {
            let name = row.get("name").cloned().unwrap_or_default();
            if !name_prefix.is_empty() && !name.starts_with(name_prefix) {
                continue;
            }
            let ros_id = row.get(".id").cloned().unwrap_or_default();
            let target = row.get("target").cloned().unwrap_or_default();
            let max_limit = row.get("max-limit").cloned().unwrap_or_default();
            let comment = row.get("comment").cloned().unwrap_or_default();

            queues.push(SimpleQueueInfo {
                ros_id,
                name,
                target,
                max_limit,
                comment,
            });
        }
        Ok(queues)
    }
}
