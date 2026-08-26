use super::api::RouterOSApiClient;
use super::rest::RouterOSRestClient;
use super::RouterOSClient;
use crate::crypto::SecretBox;
use crate::db::models::Router;
use std::time::Duration;

pub fn make_client(router: &Router, secret_key: &str, timeout_secs: Option<u64>) -> Box<dyn RouterOSClient> {
    let sbox = SecretBox::new(secret_key);
    let password = sbox.decrypt(&router.secret_enc).unwrap_or_else(|| router.secret_enc.clone());
    let timeout = Duration::from_secs(timeout_secs.unwrap_or(10));

    if router.proto == "rest" || router.proto == "rest-http" {
        let https = router.proto != "rest-http";
        let allow_scheme_fallback = router.proto == "rest";
        Box::new(RouterOSRestClient::new(
            router.host.clone(),
            router.port,
            router.username.clone(),
            password,
            router.tls_verify,
            https,
            allow_scheme_fallback,
            timeout,
        ))
    } else {
        let use_tls = router.proto != "api-plain";
        let ssl_verify = if use_tls { router.tls_verify } else { false };
        Box::new(RouterOSApiClient::new(
            router.host.clone(),
            router.port,
            router.username.clone(),
            password,
            use_tls,
            ssl_verify,
            timeout,
        ))
    }
}
