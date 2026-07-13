//! Bounded loopback-only audio and image extraction.
//!
//! This module deliberately has no chart, identity, diagnosis, or workflow
//! authority. It accepts synthetic learning media, forwards the raw bytes to
//! one fixed local Liquid model, and returns extraction text for human review.

use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const AUDIO_MODEL: &str = "LFM2.5-Audio-1.5B";
const VISION_MODEL: &str = "LFM2.5-VL-1.6B";
const MAX_AUDIO_BYTES: usize = 25 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 20_000_000;
const MAX_MODEL_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_TEXT_BYTES: usize = 8 * 1024;
const MODEL_TIMEOUT: Duration = Duration::from_secs(30);

static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
struct Capability<'a> {
    available: bool,
    model: &'a str,
    synthetic_only: bool,
    authority: &'a str,
    limits: serde_json::Value,
}

#[derive(Serialize)]
struct Extraction<'a> {
    model: &'a str,
    text: String,
    sha256: String,
    synthetic_only: bool,
    authority: &'a str,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
    message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelReply {
    text: String,
}

pub async fn capabilities_audio() -> Response {
    Json(Capability {
        available: endpoint_from_env("LIQUID_AUDIO_URL").is_ok(),
        model: AUDIO_MODEL,
        synthetic_only: true,
        authority: "transcription only; clinician review required; no diagnosis, triage, identity, or chart write",
        limits: serde_json::json!({"format":"wav","bytes":MAX_AUDIO_BYTES,"seconds":300}),
    })
    .into_response()
}

pub async fn capabilities_image() -> Response {
    Json(Capability {
        available: endpoint_from_env("LIQUID_VISION_URL").is_ok(),
        model: VISION_MODEL,
        synthetic_only: true,
        authority: "visual text extraction only; clinician review required; no diagnosis, triage, identity, or chart write",
        limits: serde_json::json!({"format":"png","bytes":MAX_IMAGE_BYTES,"pixels":MAX_IMAGE_PIXELS}),
    })
    .into_response()
}

pub async fn audio(headers: HeaderMap, body: Bytes) -> Response {
    if let Some(response) = synthetic_error(&headers) {
        return response;
    }
    if let Err(message) = validate_wav(&body) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_audio", message);
    }
    extract("LIQUID_AUDIO_URL", AUDIO_MODEL, body).await
}

pub async fn image(headers: HeaderMap, body: Bytes) -> Response {
    if let Some(response) = synthetic_error(&headers) {
        return response;
    }
    if let Err(message) = validate_png(&body) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_image", message);
    }
    extract("LIQUID_VISION_URL", VISION_MODEL, body).await
}

fn synthetic_error(headers: &HeaderMap) -> Option<Response> {
    let marked = headers
        .get("x-synthetic-workflow")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    if marked {
        None
    } else {
        Some(error(
            StatusCode::PRECONDITION_REQUIRED,
            "synthetic_workflow_required",
            "set x-synthetic-workflow: true; real patient media is not accepted",
        ))
    }
}

async fn extract(env_name: &str, model: &'static str, body: Bytes) -> Response {
    let endpoint = match endpoint_from_env(env_name) {
        Ok(endpoint) => endpoint,
        Err(message) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "model_unavailable",
                message,
            )
        }
    };
    let _guard = match InFlightGuard::acquire() {
        Some(guard) => guard,
        None => {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "model_busy",
                "one local media extraction is already running",
            )
        }
    };
    let digest = format!("sha256:{:x}", Sha256::digest(&body));
    let reply = match timeout(MODEL_TIMEOUT, post_raw(&endpoint, model, &body)).await {
        Err(_) => {
            return error(
                StatusCode::GATEWAY_TIMEOUT,
                "model_timeout",
                "the local media model exceeded 30 seconds",
            )
        }
        Ok(Err(message)) => return error(StatusCode::BAD_GATEWAY, "model_error", message),
        Ok(Ok(reply)) => reply,
    };
    if reply.text.trim().is_empty() {
        return error(
            StatusCode::BAD_GATEWAY,
            "model_output_empty",
            "the local model returned no observation text",
        );
    }
    if reply.text.len() > MAX_TEXT_BYTES {
        return error(
            StatusCode::BAD_GATEWAY,
            "model_output_too_large",
            "the local model returned more than 8192 bytes of text",
        );
    }
    Json(Extraction {
        model,
        text: reply.text,
        sha256: digest,
        synthetic_only: true,
        authority: "unsigned extraction for clinician review; not diagnosis, triage, identity, or a chart write",
    })
    .into_response()
}

struct InFlightGuard;

impl InFlightGuard {
    fn acquire() -> Option<Self> {
        IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        IN_FLIGHT.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
struct Endpoint {
    authority: String,
    connect: String,
    path: String,
}

fn endpoint_from_env(name: &str) -> Result<Endpoint, String> {
    let value = std::env::var(name).map_err(|_| format!("{name} is not configured"))?;
    parse_loopback_endpoint(&value)
}

fn parse_loopback_endpoint(value: &str) -> Result<Endpoint, String> {
    if value.chars().any(char::is_control) || value.contains('#') || value.contains('@') {
        return Err("local model endpoint is malformed".into());
    }
    let rest = value
        .strip_prefix("http://")
        .ok_or_else(|| "local model endpoint must use loopback http://".to_string())?;
    let (authority, suffix) = rest.split_once('/').unwrap_or((rest, ""));
    let path = format!("/{suffix}");
    let (host, port, connect) = if let Some(tail) = authority.strip_prefix("[::1]") {
        let port = parse_port(tail)?;
        ("[::1]", port, format!("[::1]:{port}"))
    } else {
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !port.is_empty() => (
                host,
                port.parse::<u16>()
                    .map_err(|_| "local model endpoint port is invalid".to_string())?,
            ),
            _ => (authority, 80),
        };
        if host != "127.0.0.1" && host != "localhost" {
            return Err("local model endpoint must be 127.0.0.1, localhost, or [::1]".into());
        }
        let connect_host = if host == "localhost" {
            "127.0.0.1"
        } else {
            host
        };
        (host, port, format!("{connect_host}:{port}"))
    };
    if host.is_empty() || path.contains(' ') {
        return Err("local model endpoint is malformed".into());
    }
    Ok(Endpoint {
        authority: if port == 80 {
            host.to_string()
        } else {
            format!("{host}:{port}")
        },
        connect,
        path,
    })
}

fn parse_port(tail: &str) -> Result<u16, String> {
    if tail.is_empty() {
        return Ok(80);
    }
    tail.strip_prefix(':')
        .ok_or_else(|| "local model endpoint is malformed".to_string())?
        .parse::<u16>()
        .map_err(|_| "local model endpoint port is invalid".to_string())
}

async fn post_raw(endpoint: &Endpoint, model: &str, body: &[u8]) -> Result<ModelReply, String> {
    let mut stream = TcpStream::connect(&endpoint.connect)
        .await
        .map_err(|_| "could not connect to the local media model".to_string())?;
    let request = format!(
        "POST {} HTTP/1.1\r\nhost: {}\r\ncontent-type: application/octet-stream\r\nx-liquid-model: {}\r\nx-liquid-purpose: bounded-synthetic-observation\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        endpoint.path,
        endpoint.authority,
        model,
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| "could not write to the local media model".to_string())?;
    stream
        .write_all(body)
        .await
        .map_err(|_| "could not write media to the local model".to_string())?;
    stream
        .shutdown()
        .await
        .map_err(|_| "could not finish the local model request".to_string())?;

    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|_| "could not read the local media model response".to_string())?;
        if read == 0 {
            break;
        }
        if raw.len().saturating_add(read) > MAX_MODEL_RESPONSE_BYTES {
            return Err("local media model response exceeded 65536 bytes".into());
        }
        raw.extend_from_slice(&chunk[..read]);
    }
    let split = raw
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .ok_or_else(|| "local media model returned malformed HTTP".to_string())?;
    let headers = std::str::from_utf8(&raw[..split])
        .map_err(|_| "local media model returned malformed HTTP".to_string())?;
    let status = headers.lines().next().unwrap_or_default();
    if !status.contains(" 200 ") {
        return Err("local media model returned a non-success status".into());
    }
    if headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"))
    {
        return Err("chunked local model responses are not accepted".into());
    }
    serde_json::from_slice(&raw[split + 4..])
        .map_err(|_| "local media model must return JSON with one text field".to_string())
}

fn validate_wav(body: &[u8]) -> Result<(), String> {
    if body.len() > MAX_AUDIO_BYTES {
        return Err("WAV exceeds 25 MiB".into());
    }
    if body.len() < 12 || &body[..4] != b"RIFF" || &body[8..12] != b"WAVE" {
        return Err("audio must be a RIFF/WAVE file".into());
    }
    let declared = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize + 8;
    if declared != body.len() {
        return Err("WAV size declaration does not match the body".into());
    }
    let mut offset = 12usize;
    let mut byte_rate = None;
    let mut frame_bytes = None;
    let mut data_len = None;
    while offset + 8 <= body.len() {
        let id = &body[offset..offset + 4];
        let length = u32::from_le_bytes(body[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let end = start
            .checked_add(length)
            .ok_or_else(|| "WAV chunk length overflow".to_string())?;
        if end > body.len() {
            return Err("WAV chunk extends past the body".into());
        }
        if id == b"fmt " {
            if length < 16 {
                return Err("WAV fmt chunk is too short".into());
            }
            let format = u16::from_le_bytes(body[start..start + 2].try_into().unwrap());
            let channels = u16::from_le_bytes(body[start + 2..start + 4].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(body[start + 4..start + 8].try_into().unwrap());
            let rate = u32::from_le_bytes(body[start + 8..start + 12].try_into().unwrap());
            let block_align = u16::from_le_bytes(body[start + 12..start + 14].try_into().unwrap());
            let bits = u16::from_le_bytes(body[start + 14..start + 16].try_into().unwrap());
            let expected_align = (channels as u32)
                .checked_mul(bits as u32)
                .filter(|value| value % 8 == 0)
                .map(|value| value / 8);
            let expected_rate = expected_align.and_then(|align| sample_rate.checked_mul(align));
            if format != 1
                || channels == 0
                || sample_rate == 0
                || bits == 0
                || expected_align != Some(block_align as u32)
                || expected_rate != Some(rate)
            {
                return Err("WAV must contain internally consistent uncompressed PCM".into());
            }
            byte_rate = Some(rate as u64);
            frame_bytes = Some(block_align as u64);
        } else if id == b"data" {
            data_len = Some(length as u64);
        }
        offset = end + (length & 1);
    }
    if offset != body.len() {
        return Err("WAV chunk padding is malformed".into());
    }
    let rate = byte_rate.ok_or_else(|| "WAV fmt chunk is required".to_string())?;
    let frame = frame_bytes.ok_or_else(|| "WAV frame size is required".to_string())?;
    let data = data_len.ok_or_else(|| "WAV data chunk is required".to_string())?;
    if data % frame != 0 {
        return Err("WAV data does not contain whole PCM frames".into());
    }
    if data > rate.saturating_mul(300) {
        return Err("WAV duration exceeds five minutes".into());
    }
    Ok(())
}

fn validate_png(body: &[u8]) -> Result<(), String> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if body.len() > MAX_IMAGE_BYTES {
        return Err("PNG exceeds 10 MiB".into());
    }
    if body.len() < 33 || &body[..8] != SIGNATURE {
        return Err("image must be a PNG file".into());
    }
    let ihdr_len = u32::from_be_bytes(body[8..12].try_into().unwrap()) as usize;
    if ihdr_len != 13 || &body[12..16] != b"IHDR" {
        return Err("PNG must begin with a 13-byte IHDR chunk".into());
    }
    let width = u32::from_be_bytes(body[16..20].try_into().unwrap()) as u64;
    let height = u32::from_be_bytes(body[20..24].try_into().unwrap()) as u64;
    if width == 0 || height == 0 || width.saturating_mul(height) > MAX_IMAGE_PIXELS {
        return Err("PNG dimensions must be nonzero and at most 20 megapixels".into());
    }
    let mut offset = 8usize;
    let mut saw_iend = false;
    while offset + 12 <= body.len() {
        let length = u32::from_be_bytes(body[offset..offset + 4].try_into().unwrap()) as usize;
        let kind = &body[offset + 4..offset + 8];
        let end = offset
            .checked_add(12)
            .and_then(|base| base.checked_add(length))
            .ok_or_else(|| "PNG chunk length overflow".to_string())?;
        if end > body.len() {
            return Err("PNG chunk extends past the body".into());
        }
        if !matches!(kind, b"IHDR" | b"PLTE" | b"IDAT" | b"IEND") {
            return Err(
                "PNG metadata, animation, and nonessential ancillary chunks are not accepted"
                    .into(),
            );
        }
        if kind == b"IEND" {
            if length != 0 || end != body.len() {
                return Err("PNG IEND chunk is malformed".into());
            }
            saw_iend = true;
        }
        offset = end;
    }
    if !saw_iend || offset != body.len() {
        return Err("PNG IEND chunk is required".into());
    }
    Ok(())
}

fn error(status: StatusCode, code: &'static str, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: code,
            message: message.into(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(byte_rate: u32, data_len: usize) -> Vec<u8> {
        let padding = data_len & 1;
        let total = 12 + 8 + 16 + 8 + data_len + padding;
        let mut body = Vec::with_capacity(total);
        body.extend_from_slice(b"RIFF");
        body.extend_from_slice(&((total - 8) as u32).to_le_bytes());
        body.extend_from_slice(b"WAVEfmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&(byte_rate / 2).to_le_bytes());
        body.extend_from_slice(&byte_rate.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&16u16.to_le_bytes());
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(data_len as u32).to_le_bytes());
        body.resize(total, 0);
        body
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut body = b"\x89PNG\r\n\x1a\n".to_vec();
        body.extend_from_slice(&13u32.to_be_bytes());
        body.extend_from_slice(b"IHDR");
        body.extend_from_slice(&width.to_be_bytes());
        body.extend_from_slice(&height.to_be_bytes());
        body.extend_from_slice(&[8, 2, 0, 0, 0]);
        body.extend_from_slice(&[0; 4]);
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(b"IEND");
        body.extend_from_slice(&[0; 4]);
        body
    }

    #[test]
    fn wav_duration_is_proven_from_data_and_byte_rate() {
        assert!(validate_wav(&wav(100, 30_000)).is_ok());
        assert!(validate_wav(&wav(100, 30_002))
            .unwrap_err()
            .contains("five minutes"));
        let mut wrong_size = wav(100, 10);
        wrong_size[4] = 0;
        assert!(validate_wav(&wrong_size).is_err());
    }

    #[test]
    fn png_dimensions_are_bounded() {
        assert!(validate_png(&png(4_000, 5_000)).is_ok());
        assert!(validate_png(&png(4_001, 5_000)).is_err());
        assert!(validate_png(b"not png").is_err());
    }

    #[test]
    fn endpoint_accepts_only_literal_loopback_hosts() {
        assert!(parse_loopback_endpoint("http://127.0.0.1:8080/infer").is_ok());
        assert!(parse_loopback_endpoint("http://localhost/infer").is_ok());
        assert!(parse_loopback_endpoint("http://[::1]:8080/infer").is_ok());
        assert!(parse_loopback_endpoint("http://10.0.0.2/infer").is_err());
        assert!(parse_loopback_endpoint("http://127.0.0.1.example/infer").is_err());
        assert!(parse_loopback_endpoint("https://127.0.0.1/infer").is_err());
    }

    #[tokio::test]
    async fn raw_media_reaches_only_the_selected_loopback_model() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            stream.read_to_end(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("POST /infer HTTP/1.1\r\n"));
            assert!(request.contains("x-liquid-model: LFM2.5-Audio-1.5B"));
            assert!(request.ends_with("synthetic wav bytes"));
            let body = r#"{"text":"bounded local transcript"}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let endpoint = parse_loopback_endpoint(&format!("http://{address}/infer")).unwrap();
        let reply = post_raw(&endpoint, AUDIO_MODEL, b"synthetic wav bytes")
            .await
            .unwrap();
        assert_eq!(reply.text, "bounded local transcript");
        server.await.unwrap();
    }
}
