use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct VerifiedSession {
    pub principal_id: String,
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    sid: String,
    exp: u64,
    iat: u64,
    #[serde(default)]
    nbf: Option<u64>,
    #[serde(default)]
    azp: Option<String>,
}

#[derive(Debug)]
struct CachedKeys {
    fetched_at: Instant,
    keys: JwkSet,
}

#[derive(Debug)]
pub struct ClerkVerifier {
    issuer: String,
    jwks_url: String,
    authorized_parties: Vec<String>,
    subjects: HashMap<String, String>,
    development_default: Option<String>,
    client: reqwest::Client,
    cache: Arc<RwLock<Option<CachedKeys>>>,
}

impl ClerkVerifier {
    pub fn from_env() -> Result<Option<Self>> {
        let issuer = match std::env::var("CLERK_ISSUER")
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            Some(value) => value.trim_end_matches('/').to_string(),
            None => return Ok(None),
        };
        if !issuer.starts_with("https://") {
            bail!("CLERK_ISSUER must use HTTPS");
        }
        let jwks_url = std::env::var("CLERK_JWKS_URL")
            .unwrap_or_else(|_| format!("{issuer}/.well-known/jwks.json"));
        if !jwks_url.starts_with("https://") {
            bail!("CLERK_JWKS_URL must use HTTPS");
        }
        let authorized_parties = split_list("CLERK_AUTHORIZED_PARTIES")?;
        if authorized_parties.is_empty() {
            bail!("CLERK_AUTHORIZED_PARTIES must name at least one exact origin");
        }
        let subjects = parse_subjects(&std::env::var("CLERK_SUBJECT_MAP").unwrap_or_default())?;
        let development_default = std::env::var("CLERK_DEVELOPMENT_DEFAULT_PRINCIPAL")
            .ok()
            .filter(|value| !value.trim().is_empty());
        if development_default.is_some() && !issuer.ends_with(".clerk.accounts.dev") {
            bail!("CLERK_DEVELOPMENT_DEFAULT_PRINCIPAL is allowed only with a Clerk development issuer");
        }
        if subjects.is_empty() && development_default.is_none() {
            bail!("Clerk mode needs CLERK_SUBJECT_MAP or a development default principal");
        }
        let client = reqwest::Client::builder()
            .https_only(true)
            .timeout(Duration::from_secs(5))
            .build()
            .context("building Clerk JWKS client")?;
        Ok(Some(Self {
            issuer,
            jwks_url,
            authorized_parties,
            subjects,
            development_default,
            client,
            cache: Arc::new(RwLock::new(None)),
        }))
    }

    pub async fn verify(&self, token: &str) -> Result<VerifiedSession> {
        if token.len() > 16_384 {
            bail!("session token is too large");
        }
        let header = decode_header(token).context("invalid session token header")?;
        if header.alg != Algorithm::RS256 {
            bail!("session token must use RS256");
        }
        let kid = header.kid.context("session token has no key id")?;
        let key = match self.key(&kid, false).await? {
            Some(key) => key,
            None => self
                .key(&kid, true)
                .await?
                .context("session token key is not trusted")?,
        };
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        validation.validate_aud = false;
        validation.leeway = 5;
        let claims = decode::<Claims>(token, &key, &validation)
            .context("session token did not verify")?
            .claims;
        if claims.iss != self.issuer {
            bail!("session token issuer is not trusted");
        }
        let party = claims
            .azp
            .context("session token has no authorized party")?;
        if !self.authorized_parties.iter().any(|item| item == &party) {
            bail!("session token authorized party is not allowed");
        }
        if claims.sub.is_empty() || claims.sid.is_empty() {
            bail!("session token is missing its subject or session id");
        }
        if claims.exp.saturating_sub(claims.iat) > 3_600 {
            bail!("session token lifetime is too long");
        }
        if claims.nbf.is_some_and(|nbf| nbf > claims.exp) {
            bail!("session token time range is invalid");
        }
        let principal_id = self
            .subjects
            .get(&claims.sub)
            .cloned()
            .or_else(|| self.development_default.clone())
            .context("Clerk user is not provisioned for this service")?;
        Ok(VerifiedSession {
            principal_id,
            session_id: claims.sid,
        })
    }

    async fn key(&self, kid: &str, refresh: bool) -> Result<Option<DecodingKey>> {
        if !refresh {
            let cache = self.cache.read().await;
            if let Some(cache) = cache.as_ref() {
                if cache.fetched_at.elapsed() < Duration::from_secs(300) {
                    if let Some(jwk) = cache.keys.find(kid) {
                        return DecodingKey::from_jwk(jwk)
                            .map(Some)
                            .context("Clerk key is invalid");
                    }
                }
            }
        }
        let keys = self
            .client
            .get(&self.jwks_url)
            .send()
            .await
            .context("fetching Clerk keys")?
            .error_for_status()
            .context("Clerk keys endpoint failed")?
            .json::<JwkSet>()
            .await
            .context("reading Clerk keys")?;
        let result = keys
            .find(kid)
            .map(DecodingKey::from_jwk)
            .transpose()
            .context("Clerk key is invalid")?;
        *self.cache.write().await = Some(CachedKeys {
            fetched_at: Instant::now(),
            keys,
        });
        Ok(result)
    }
}

fn split_list(name: &str) -> Result<Vec<String>> {
    Ok(std::env::var(name)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect())
}

fn parse_subjects(value: &str) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    for pair in value
        .split(',')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
    {
        let (subject, principal) = pair
            .split_once('=')
            .context("CLERK_SUBJECT_MAP entries must be subject=principal")?;
        if subject.trim().is_empty() || principal.trim().is_empty() {
            bail!("CLERK_SUBJECT_MAP contains an empty subject or principal");
        }
        result.insert(subject.trim().to_string(), principal.trim().to_string());
    }
    Ok(result)
}
