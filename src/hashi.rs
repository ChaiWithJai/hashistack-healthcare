//! Staging HashiStack clients (#2): a Nomad job submitter and a Vault transit
//! prober, active only when `NOMAD_ADDR` / `VAULT_ADDR`+`VAULT_TOKEN` are set.
//! With neither present the control plane keeps its simulated semantics and
//! this module is never exercised — no test or demo path depends on it.
//!
//! The HTTP client is deliberately hand-rolled over `std::net`: staging talks
//! plain HTTP/1.0 to dev agents on localhost, and a dependency-free ~60-line
//! client beats pulling a TLS-capable crate into the tree for that. HTTP/1.0
//! with `connection: close` means responses are never chunked — read to EOF
//! and split headers from body.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// A dynamic database credential lease from Vault's database engine (#9).
///
/// The password is deliberately quarantined: private field, redacted
/// `Debug`, no `Serialize`, and [`DbLease::audit_detail`] is built only
/// from the non-secret fields — the password exists in memory exactly long
/// enough to prove the credential authenticates, and is never recorded on
/// the allocation, the audit stream, or any durable surface.
pub struct DbLease {
    pub lease_id: String,
    pub username: String,
    pub ttl_secs: u64,
    password: String,
}

impl DbLease {
    pub fn password(&self) -> &str {
        &self.password
    }

    /// The audit-safe record of this lease. Lease id and username are not
    /// sensitive (they are revocation/inspection handles); the password is
    /// structurally absent — this method cannot see it into the string.
    pub fn audit_detail(&self) -> String {
        format!(
            "lease {} issued: user {}, ttl {}s (password held in memory only, never recorded)",
            self.lease_id, self.username, self.ttl_secs
        )
    }
}

impl std::fmt::Debug for DbLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbLease")
            .field("lease_id", &self.lease_id)
            .field("username", &self.username)
            .field("ttl_secs", &self.ttl_secs)
            .field("password", &"<redacted>")
            .finish()
    }
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(15);

/// One HTTP/1.0 exchange. Returns (status, body). Plain http only — staging
/// dev agents listen on localhost without TLS; anything else should go
/// through a real client in a real deployment, not this module.
fn http(
    base: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> Result<(u16, String)> {
    let authority = base
        .trim_end_matches('/')
        .strip_prefix("http://")
        .ok_or_else(|| {
            anyhow!("staging clients speak plain http to local dev agents, got {base}")
        })?;
    let addr = authority
        .to_socket_addrs()
        .with_context(|| format!("resolving {authority}"))?
        .next()
        .ok_or_else(|| anyhow!("{authority} resolved to no address"))?;
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .with_context(|| format!("connecting to {authority}"))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;

    let mut req = format!(
        "{method} {path} HTTP/1.0\r\nhost: {authority}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    if let Some(token) = token {
        req.push_str(&format!("x-vault-token: {token}\r\n"));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes())?;
    stream.write_all(body.as_bytes())?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .with_context(|| format!("reading response from {authority}"))?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed response from {authority}"))?;
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("malformed status line from {authority}: {head}"))?;
    Ok((status, body.to_string()))
}

fn expect_ok(what: &str, status: u16, body: &str) -> Result<()> {
    if !(200..300).contains(&status) {
        bail!("{what} failed: HTTP {status} — {}", body.trim());
    }
    Ok(())
}

// ---------- nomad ----------

/// A real Nomad dev agent, when `NOMAD_ADDR` is set.
pub struct Nomad {
    addr: String,
}

impl Nomad {
    pub fn from_env() -> Option<Self> {
        std::env::var("NOMAD_ADDR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|addr| Self { addr })
    }

    /// Namespaces are records, not conventions: the job pins
    /// `namespace = "tenant-<tenant>"`, so the namespace must exist before
    /// registration. This is the upsert endpoint — idempotent by design.
    pub fn ensure_namespace(&self, namespace: &str) -> Result<()> {
        let body = json!({ "Name": namespace, "Description": "per-tenant staging namespace" });
        let (status, resp) = http(&self.addr, "POST", "/v1/namespace", None, &body.to_string())?;
        expect_ok("nomad namespace upsert", status, &resp)
    }

    /// Submit rendered job HCL: parse it server-side (/v1/jobs/parse), then
    /// register the parsed job (/v1/jobs). Returns the evaluation id — the
    /// receipt that Nomad, not this process, now owns the scheduling story.
    ///
    /// One staging-only adjustment: the dev agent has no Vault workload
    /// identity configured, so the job's `vault` stanza is stripped before
    /// registration (the control plane proves the Vault side itself via the
    /// transit probe at promote). Cloud staging keeps the stanza.
    pub fn submit_job_hcl(&self, job_hcl: &str) -> Result<String> {
        let parse_body = json!({ "JobHCL": job_hcl, "Canonicalize": true });
        let (status, resp) = http(
            &self.addr,
            "POST",
            "/v1/jobs/parse",
            None,
            &parse_body.to_string(),
        )?;
        expect_ok("nomad job parse", status, &resp)?;
        let mut job: Value = serde_json::from_str(&resp).context("parsing nomad job JSON")?;

        if let Some(groups) = job.get_mut("TaskGroups").and_then(Value::as_array_mut) {
            for group in groups {
                if let Some(tasks) = group.get_mut("Tasks").and_then(Value::as_array_mut) {
                    for task in tasks {
                        if let Some(task) = task.as_object_mut() {
                            task.remove("Vault");
                        }
                    }
                }
            }
        }

        let register_body = json!({ "Job": job });
        let (status, resp) = http(
            &self.addr,
            "POST",
            "/v1/jobs",
            None,
            &register_body.to_string(),
        )?;
        expect_ok("nomad job register", status, &resp)?;
        let receipt: Value =
            serde_json::from_str(&resp).context("parsing nomad register response")?;
        receipt
            .get("EvalID")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("nomad register returned no EvalID: {resp}"))
    }

    /// Observed job status (#6, the honest slice of real allocations):
    /// `GET /v1/job/:id` and return Nomad's own `Status` word (`pending` |
    /// `running` | `dead`) — the OBSERVED axis of Nomad's desired/observed
    /// pair (steering: `structs`, desired vs observed status). On the
    /// one-machine dev agent the `role=prod` constraint is unsatisfiable,
    /// so an honest `pending` is exactly what this reports; per-allocation
    /// ClientStatus and deployment health ride the real client pool
    /// (Phase 1), together with release≠deploy and generations.
    pub fn job_status(&self, job_id: &str, namespace: &str) -> Result<String> {
        let path = format!("/v1/job/{job_id}?namespace={namespace}");
        let (status, resp) = http(&self.addr, "GET", &path, None, "")?;
        expect_ok("nomad job status", status, &resp)?;
        let v: Value = serde_json::from_str(&resp).context("parsing nomad job response")?;
        v["Status"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("nomad job read returned no Status for {job_id}"))
    }

    /// Stop (not purge) the job: rollback destroys the allocation but keeps
    /// the record inspectable — `nomad job status` shows it dead, which is
    /// exactly what the pressure test asserts.
    pub fn stop_job(&self, job_id: &str, namespace: &str) -> Result<()> {
        let path = format!("/v1/job/{job_id}?namespace={namespace}");
        let (status, resp) = http(&self.addr, "DELETE", &path, None, "")?;
        expect_ok("nomad job stop", status, &resp)
    }
}

// ---------- vault ----------

/// A real Vault dev server, when `VAULT_ADDR` and `VAULT_TOKEN` are both set.
pub struct Vault {
    addr: String,
    token: String,
}

impl Vault {
    pub fn from_env() -> Option<Self> {
        let addr = std::env::var("VAULT_ADDR")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let token = std::env::var("VAULT_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        Some(Self { addr, token })
    }

    /// Prove the tenant's transit key end-to-end: create `transit/keys/<key>`
    /// if missing, then encrypt/decrypt a probe and demand the plaintext back.
    /// This makes the gate's "encryption keys: Vault" line an exercised fact,
    /// not a string literal.
    pub fn transit_roundtrip(&self, key: &str, probe: &str) -> Result<()> {
        let token = Some(self.token.as_str());
        // Upsert the key. Vault answers 204 for a fresh key and 400 if the key
        // exists with differing (default) params — both mean "key present".
        let path = format!("/v1/transit/keys/{key}");
        let (status, resp) = http(&self.addr, "POST", &path, token, "{}")?;
        if !(200..300).contains(&status) && status != 400 {
            bail!(
                "vault transit key upsert failed: HTTP {status} — {}",
                resp.trim()
            );
        }

        let plaintext_b64 = base64(probe.as_bytes());
        let body = json!({ "plaintext": plaintext_b64 }).to_string();
        let (status, resp) = http(
            &self.addr,
            "POST",
            &format!("/v1/transit/encrypt/{key}"),
            token,
            &body,
        )?;
        expect_ok("vault transit encrypt", status, &resp)?;
        let ciphertext = serde_json::from_str::<Value>(&resp)
            .ok()
            .and_then(|v| v["data"]["ciphertext"].as_str().map(str::to_string))
            .ok_or_else(|| anyhow!("vault encrypt returned no ciphertext"))?;

        let body = json!({ "ciphertext": ciphertext }).to_string();
        let (status, resp) = http(
            &self.addr,
            "POST",
            &format!("/v1/transit/decrypt/{key}"),
            token,
            &body,
        )?;
        expect_ok("vault transit decrypt", status, &resp)?;
        let roundtrip = serde_json::from_str::<Value>(&resp)
            .ok()
            .and_then(|v| v["data"]["plaintext"].as_str().map(str::to_string))
            .ok_or_else(|| anyhow!("vault decrypt returned no plaintext"))?;
        if roundtrip != plaintext_b64 {
            bail!("vault transit round-trip corrupted the probe for key {key}");
        }
        Ok(())
    }

    /// Request dynamic Postgres credentials from the database engine (#9):
    /// `GET /v1/database/creds/<role>`. Vault creates the role in Postgres
    /// on the spot; the returned lease is the revocation handle.
    pub fn db_creds(&self, role: &str) -> Result<DbLease> {
        let path = format!("/v1/database/creds/{role}");
        let (status, resp) = http(&self.addr, "GET", &path, Some(&self.token), "")?;
        expect_ok("vault database creds", status, &resp)?;
        let v: Value = serde_json::from_str(&resp).context("parsing vault creds response")?;
        let lease_id = v["lease_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("vault creds returned no lease_id"))?
            .to_string();
        let username = v["data"]["username"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("vault creds returned no username"))?
            .to_string();
        let password = v["data"]["password"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("vault creds returned no password"))?
            .to_string();
        let ttl_secs = v["lease_duration"]
            .as_u64()
            .ok_or_else(|| anyhow!("vault creds returned no lease_duration"))?;
        Ok(DbLease {
            lease_id,
            username,
            ttl_secs,
            password,
        })
    }

    /// Revoke a lease (`PUT /v1/sys/leases/revoke`): the database engine
    /// runs its revocation statements, dropping the issued role.
    pub fn revoke_lease(&self, lease_id: &str) -> Result<()> {
        let body = json!({ "lease_id": lease_id }).to_string();
        let (status, resp) = http(
            &self.addr,
            "PUT",
            "/v1/sys/leases/revoke",
            Some(&self.token),
            &body,
        )?;
        expect_ok("vault lease revoke", status, &resp)
    }

    /// Read an ACL policy back (`GET /v1/sys/policies/acl/<name>`).
    /// `Ok(None)` means the policy does not exist yet.
    pub fn policy_read(&self, name: &str) -> Result<Option<String>> {
        let path = format!("/v1/sys/policies/acl/{name}");
        let (status, resp) = http(&self.addr, "GET", &path, Some(&self.token), "")?;
        if status == 404 {
            return Ok(None);
        }
        expect_ok("vault policy read", status, &resp)?;
        let v: Value = serde_json::from_str(&resp).context("parsing vault policy response")?;
        Ok(v["data"]["policy"].as_str().map(str::to_string))
    }

    /// Write an ACL policy (`PUT /v1/sys/policies/acl/<name>`) — the
    /// per-tenant policy mount (#9). Idempotent by overwrite.
    pub fn policy_write(&self, name: &str, policy: &str) -> Result<()> {
        let path = format!("/v1/sys/policies/acl/{name}");
        let body = json!({ "policy": policy }).to_string();
        let (status, resp) = http(&self.addr, "PUT", &path, Some(&self.token), &body)?;
        expect_ok("vault policy write", status, &resp)
    }
}

/// Standard base64 (RFC 4648, padded). Hand-rolled so the transit probe costs
/// zero dependencies; probes are tens of bytes, performance is irrelevant.
fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{base64, DbLease};

    /// #9's do-NOT-log-the-password rule, asserted by grepping the audit
    /// export: a lease with a known password is recorded through the same
    /// `audit_detail` path staging uses, and the export (plus the tenant
    /// view and Debug) must never contain the secret.
    #[test]
    fn db_lease_password_never_reaches_any_audit_surface() {
        let secret = ["unit-test-only-", "vault-password-sentinel"].concat();
        let lease = DbLease {
            lease_id: "database/creds/tenant-app/AbC123".to_string(),
            username: "v-token-tenant-ap-xyz".to_string(),
            ttl_secs: 3600,
            password: secret.clone(),
        };

        assert!(
            !format!("{lease:?}").contains(&secret),
            "Debug must redact the password"
        );
        assert!(lease.audit_detail().contains(&lease.lease_id));
        assert!(lease.audit_detail().contains(&lease.username));
        assert!(!lease.audit_detail().contains(&secret));

        let mut log = crate::audit::AuditLog::default();
        log.record(
            "deploy",
            "vault.db_creds_issued",
            lease.audit_detail(),
            Some("app-1"),
        );
        let export = log.export_jsonl();
        assert!(
            export.contains("vault.db_creds_issued") && !export.contains(&secret),
            "audit export must carry the event but never the password: {export}"
        );
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
