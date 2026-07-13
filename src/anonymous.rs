use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

const SECURE_COOKIE: &str = "__Host-hashistack_guest";
const LOCAL_COOKIE: &str = "hashistack_guest";
const TTL_SECS: u64 = 86_400;

#[derive(Debug)]
pub struct AnonymousSessions {
    key: Vec<u8>,
    secure: bool,
    allowed_origins: Vec<String>,
    netlify_preview_site: Option<String>,
}

impl AnonymousSessions {
    pub fn development() -> Self {
        Self {
            key: b"development-only-anonymous-workspace-key".to_vec(),
            secure: false,
            allowed_origins: Vec::new(),
            netlify_preview_site: None,
        }
    }

    pub fn from_env(deployed: bool) -> Result<Self> {
        let key = std::env::var("ANON_SESSION_HMAC_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let key = match key {
            Some(value) if value.len() >= 32 => value.into_bytes(),
            Some(_) => bail!("ANON_SESSION_HMAC_KEY must contain at least 32 bytes"),
            None if deployed => bail!("ANON_SESSION_HMAC_KEY is required with Clerk"),
            None => return Ok(Self::development()),
        };
        let allowed_origins = std::env::var("CLERK_AUTHORIZED_PARTIES")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();
        let netlify_preview_site = std::env::var("ANON_NETLIFY_PREVIEW_SITE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                let value = value.trim().to_string();
                if !valid_netlify_site_name(&value) {
                    bail!("ANON_NETLIFY_PREVIEW_SITE must be a lowercase Netlify site name");
                }
                Ok(value)
            })
            .transpose()?;
        Ok(Self {
            key,
            secure: deployed,
            allowed_origins,
            netlify_preview_site,
        })
    }

    pub fn issue(&self) -> Result<(String, String)> {
        let mut id = [0_u8; 32];
        getrandom::getrandom(&mut id)
            .map_err(|error| anyhow::anyhow!("creating anonymous workspace: {error}"))?;
        let issued = now();
        let expires = issued + TTL_SECS;
        let mut payload = Vec::with_capacity(48);
        payload.extend_from_slice(&id);
        payload.extend_from_slice(&issued.to_be_bytes());
        payload.extend_from_slice(&expires.to_be_bytes());
        let encoded = URL_SAFE_NO_PAD.encode(&payload);
        let signature = self.sign(encoded.as_bytes())?;
        let value = format!("v1.{encoded}.{}", URL_SAFE_NO_PAD.encode(signature));
        let secure = if self.secure { "; Secure" } else { "" };
        let name = if self.secure {
            SECURE_COOKIE
        } else {
            LOCAL_COOKIE
        };
        let cookie =
            format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={TTL_SECS}{secure}");
        Ok((cookie, tenant_for(&id)))
    }

    pub fn tenant_from_cookie(&self, cookie_header: &str) -> Result<String> {
        let value = cookie_header
            .split(';')
            .map(str::trim)
            .find_map(|part| {
                part.strip_prefix(&format!("{SECURE_COOKIE}="))
                    .or_else(|| part.strip_prefix(&format!("{LOCAL_COOKIE}=")))
            })
            .context("anonymous workspace cookie is missing")?;
        let mut parts = value.split('.');
        if parts.next() != Some("v1") {
            bail!("anonymous workspace cookie version is invalid");
        }
        let encoded = parts
            .next()
            .context("anonymous workspace payload is missing")?;
        let signature = parts
            .next()
            .context("anonymous workspace signature is missing")?;
        if parts.next().is_some() {
            bail!("anonymous workspace cookie is malformed");
        }
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .context("anonymous workspace signature is malformed")?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key)
            .context("anonymous workspace key is invalid")?;
        mac.update(encoded.as_bytes());
        mac.verify_slice(&signature)
            .context("anonymous workspace signature is invalid")?;
        let payload = URL_SAFE_NO_PAD
            .decode(encoded)
            .context("anonymous workspace payload is malformed")?;
        if payload.len() != 48 {
            bail!("anonymous workspace payload length is invalid");
        }
        let id: [u8; 32] = payload[..32].try_into().unwrap();
        let issued = u64::from_be_bytes(payload[32..40].try_into().unwrap());
        let expires = u64::from_be_bytes(payload[40..48].try_into().unwrap());
        let current = now();
        if issued > current + 5 || expires < current || expires.saturating_sub(issued) != TTL_SECS {
            bail!("anonymous workspace cookie is expired or invalid");
        }
        Ok(tenant_for(&id))
    }

    pub fn origin_allowed(&self, origin: Option<&str>, fetch_site: Option<&str>) -> bool {
        if fetch_site.is_some_and(|value| value != "same-origin" && value != "none") {
            return false;
        }
        if self.allowed_origins.is_empty() {
            return true;
        }
        origin.is_some_and(|value| {
            self.allowed_origins.iter().any(|item| item == value)
                || self
                    .netlify_preview_site
                    .as_deref()
                    .is_some_and(|site| netlify_preview_origin(value, site))
        })
    }

    fn sign(&self, value: &[u8]) -> Result<Vec<u8>> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key)
            .context("anonymous workspace key is invalid")?;
        mac.update(value);
        Ok(mac.finalize().into_bytes().to_vec())
    }
}

fn valid_netlify_site_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn netlify_preview_origin(origin: &str, site: &str) -> bool {
    let Some(pr) = origin.strip_prefix("https://deploy-preview-") else {
        return false;
    };
    let suffix = format!("--{site}.netlify.app");
    let Some(pr) = pr.strip_suffix(&suffix) else {
        return false;
    };
    !pr.is_empty() && pr.bytes().all(|byte| byte.is_ascii_digit())
}

fn tenant_for(id: &[u8; 32]) -> String {
    let digest = Sha256::digest(id);
    let mut value = String::from("anon-");
    for byte in &digest[..16] {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod preview_origin_tests {
    use super::{netlify_preview_origin, valid_netlify_site_name};

    #[test]
    fn accepts_only_the_numbered_preview_for_the_configured_site() {
        assert!(netlify_preview_origin(
            "https://deploy-preview-45--gethoursback.netlify.app",
            "gethoursback"
        ));
        for origin in [
            "http://deploy-preview-45--gethoursback.netlify.app",
            "https://deploy-preview-main--gethoursback.netlify.app",
            "https://deploy-preview-45--other.netlify.app",
            "https://deploy-preview-45--gethoursback.netlify.app.evil.example",
            "https://gethoursback.netlify.app",
        ] {
            assert!(!netlify_preview_origin(origin, "gethoursback"), "{origin}");
        }
    }

    #[test]
    fn site_name_cannot_escape_the_netlify_hostname() {
        assert!(valid_netlify_site_name("gethoursback"));
        assert!(valid_netlify_site_name("practice-studio-2"));
        for value in [
            "",
            "-site",
            "site-",
            "Site",
            "site.netlify.app",
            "site/evil",
        ] {
            assert!(!valid_netlify_site_name(value), "{value}");
        }
    }
}
