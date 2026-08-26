use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::engine::general_purpose::URL_SAFE;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
type HmacSha256 = Hmac<Sha256>;

pub struct SecretBox {
    signing_key: [u8; 16],
    encryption_key: [u8; 16],
}

impl SecretBox {
    pub fn new(secret: &str) -> Self {
        let digest = Sha256::digest(secret.as_bytes());
        let mut signing_key = [0u8; 16];
        let mut encryption_key = [0u8; 16];
        signing_key.copy_from_slice(&digest[0..16]);
        encryption_key.copy_from_slice(&digest[16..32]);

        Self {
            signing_key,
            encryption_key,
        }
    }

    pub fn encrypt(&self, plaintext: &str) -> String {
        let mut iv = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut iv);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut data = Vec::with_capacity(1 + 8 + 16 + plaintext.len() + 16 + 32);
        data.push(0x80); // Fernet version
        data.extend_from_slice(&now.to_be_bytes());
        data.extend_from_slice(&iv);

        // AES-128-CBC encryption
        let mut buf = vec![0u8; plaintext.len() + 16];
        buf[..plaintext.len()].copy_from_slice(plaintext.as_bytes());
        let encryptor = Aes128CbcEnc::new(&self.encryption_key.into(), &iv.into());
        let ct = encryptor
            .encrypt_padded_mut::<Pkcs7>(&mut buf, plaintext.len())
            .expect("padding failed");
        data.extend_from_slice(ct);

        // HMAC-SHA256
        let mut mac = HmacSha256::new_from_slice(&self.signing_key).expect("hmac init");
        mac.update(&data);
        let tag = mac.finalize().into_bytes();
        data.extend_from_slice(&tag);

        URL_SAFE.encode(data)
    }

    pub fn decrypt(&self, token: &str) -> Option<String> {
        let raw = URL_SAFE.decode(token.trim()).ok()?;
        if raw.len() < 73 || raw[0] != 0x80 {
            return None;
        }

        let tag_start = raw.len() - 32;
        let (payload, received_tag) = raw.split_at(tag_start);

        let mut mac = HmacSha256::new_from_slice(&self.signing_key).ok()?;
        mac.update(payload);
        if mac.verify_slice(received_tag).is_err() {
            return None;
        }

        let iv = &payload[9..25];
        let ciphertext = &payload[25..];

        let mut buf = ciphertext.to_vec();
        let decryptor = Aes128CbcDec::new(&self.encryption_key.into(), iv.into());
        let pt = decryptor.decrypt_padded_mut::<Pkcs7>(&mut buf).ok()?;

        String::from_utf8(pt.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fernet_roundtrip() {
        let secret = "my-test-secret-key-12345";
        let box_sec = SecretBox::new(secret);
        let msg = "Hello MikroTik WireGuard!";
        let enc = box_sec.encrypt(msg);
        let dec = box_sec.decrypt(&enc);
        assert_eq!(dec, Some(msg.to_string()));
    }
}
