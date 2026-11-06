//! What the desktop gives the Assistant's model endpoints: HTTPS and the system keychain. Adapted
//! from SolidCraft's `apps/solidcraft/src/main.rs` (`agent_http` and `Keychain`, see
//! `ATTRIBUTION.md`).

use craft_agents::backend::{HttpRequest, Secrets};

/// The keychain service PCBCraft's endpoint keys are stored under, one entry per agent.
const SERVICE: &str = "PCBCraft Assistant";
/// Most bytes read back from one response.
const MAX_RESPONSE: u64 = 16 << 20;

/// HTTP for model endpoints: to the URL the person configured, which may be plain HTTP on this
/// computer (Ollama, LM Studio). No redirects (a key must not follow one elsewhere), no cookies,
/// a whole-request timeout and at most 16 MB back. Any HTTP status is returned as an answer.
pub(super) fn http(request: &HttpRequest) -> Result<(u16, Vec<u8>), String> {
    use std::time::Duration;
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(request.timeout_s.clamp(1, 3600))))
        .timeout_connect(Some(Duration::from_secs(10)))
        .user_agent("PCBCraft")
        .max_redirects(0)
        .build()
        .into();
    let mut response = if request.method == "POST" {
        let mut call = agent.post(&request.url);
        for (name, value) in &request.headers {
            call = call.header(name, value);
        }
        call.send(request.body.as_deref().unwrap_or_default())
    } else {
        let mut call = agent.get(&request.url);
        for (name, value) in &request.headers {
            call = call.header(name, value);
        }
        call.call()
    }
    .map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let body = response.body_mut().with_config().limit(MAX_RESPONSE).read_to_vec().map_err(|error| error.to_string())?;
    Ok((status, body))
}

/// Endpoint keys in the system keychain, never in PCBCraft's settings.
pub(super) struct Keychain;

impl Keychain {
    /// The keychain entry for agent `id`.
    fn entry(id: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, id).map_err(|error| error.to_string())
    }
}

impl Secrets for Keychain {
    /// The stored key, or `None` when there is none.
    fn get(&self, id: &str) -> Result<Option<String>, String> {
        match Self::entry(id)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set(&self, id: &str, secret: &str) -> Result<(), String> {
        Self::entry(id)?.set_password(secret).map_err(|error| error.to_string())
    }

    /// Remove the key; a missing one is already removed.
    fn delete(&self, id: &str) -> Result<(), String> {
        match Self::entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}
