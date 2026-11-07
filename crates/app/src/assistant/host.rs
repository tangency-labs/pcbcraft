//! What the desktop gives the Assistant's model endpoints: HTTPS and the system keychain. Adapted
//! from SolidCraft's `apps/solidcraft/src/main.rs` (`agent_http` and `Keychain`, see
//! `ATTRIBUTION.md`).

use craft_agents::backend::{HttpRequest, Secrets};

/// The keychain service PCBCraft's endpoint keys are stored under, one entry per agent.
const SERVICE: &str = "PCBCraft Assistant";
/// Most bytes read back from one response.
const MAX_RESPONSE: u64 = 16 << 20;

/// A key as the keychain holds it: with the endpoint address it was entered for. The settings
/// that name the address are saved separately (and later), so a key is never trusted for another
/// address, even after a crash between the two saves.
fn bind_key(base_url: &str, key: &str) -> String {
    serde_json::json!({"base_url": base_url, "key": key}).to_string()
}

/// The key in a stored secret when it was entered for `base_url`; otherwise why it isn't used.
/// Never names the secret.
fn key_for(secret: &str, base_url: &str) -> Result<String, String> {
    let stored: serde_json::Value = serde_json::from_str(secret).unwrap_or_default();
    match (stored["base_url"].as_str(), stored["key"].as_str()) {
        (Some(url), Some(key)) if url == base_url && !key.is_empty() => Ok(key.to_owned()),
        _ => Err("The stored key was entered for another address. Edit the endpoint in Preferences › AI to enter it again.".to_owned()),
    }
}

/// The key stored for endpoint `id`, only when it was entered for `base_url`; `Ok(None)` when
/// none is stored.
pub(super) fn endpoint_key(id: &str, base_url: &str) -> Result<Option<String>, String> {
    let secret = Keychain.get(id).map_err(|error| format!("Could not read the key from the keychain: {error}"))?;
    secret.map(|secret| key_for(&secret, base_url)).transpose()
}

/// Store `key` for endpoint `id`, bound to `base_url`.
pub(super) fn store_endpoint_key(id: &str, base_url: &str, key: &str) -> Result<(), String> {
    Keychain.set(id, &bind_key(base_url, key)).map_err(|error| format!("Could not store the key in the keychain: {error}"))
}

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

    /// Store or replace the key in the system keychain.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A key is only used for the address it was entered for: after an edit to another endpoint
    /// whose settings were never saved, the old address finds a key it refuses. A key stored
    /// without its address, as before, is refused too; neither error shows the key.
    #[test]
    fn a_key_is_only_used_for_the_address_it_was_entered_for() {
        let stored = bind_key("https://b.example/v1", "sk-b");
        assert_eq!(key_for(&stored, "https://b.example/v1"), Ok("sk-b".to_owned()));
        for (secret, url) in [(stored.as_str(), "https://a.example/v1"), ("sk-plain", "https://b.example/v1"), ("", "https://b.example/v1")] {
            let error = key_for(secret, url).err().unwrap_or_default();
            assert!(error.contains("another address") && !error.contains("sk-"), "{error}");
        }
    }
}
