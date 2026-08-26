use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rand::RngCore;
use x25519_dalek::{PublicKey, StaticSecret};

pub fn generate_wireguard_keypair() -> (String, String) {
    let mut rng = rand::thread_rng();
    let mut secret_bytes = [0u8; 32];
    rng.fill_bytes(&mut secret_bytes);

    let secret = StaticSecret::from(secret_bytes);
    let public = PublicKey::from(&secret);

    let private_b64 = BASE64.encode(secret.to_bytes());
    let public_b64 = BASE64.encode(public.as_bytes());

    (private_b64, public_b64)
}

pub fn generate_preshared_key() -> String {
    let mut psk = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut psk);
    BASE64.encode(psk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wireguard_keypair_generation() {
        let (priv_key, pub_key) = generate_wireguard_keypair();
        assert_eq!(priv_key.len(), 44);
        assert_eq!(pub_key.len(), 44);
        assert!(priv_key.ends_with('='));
        assert!(pub_key.ends_with('='));
    }
}
