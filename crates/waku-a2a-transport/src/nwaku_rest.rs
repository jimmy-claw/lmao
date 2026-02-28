//! nwaku REST API transport — fallback implementation.
//!
//! Talks to a running nwaku node via its REST API (default: http://localhost:8645).
//! This is the v0.1 transport while we work on the logos-delivery-rust-bindings FFI
//! integration (Issue #1).

use crate::Transport;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Transport implementation backed by the nwaku REST API.
///
/// Requires a running nwaku node. Start one with:
/// ```bash
/// docker run -p 8645:8645 statusteam/nim-waku:v0.31.0 \
///   --rest --rest-address=0.0.0.0 --rest-port=8645
/// ```
pub struct NwakuTransport {
    waku_url: String,
    client: reqwest::Client,
    subscriptions: Mutex<HashMap<String, JoinHandle<()>>>,
}

#[derive(Serialize)]
struct RelayMessage {
    payload: String,
    #[serde(rename = "contentTopic")]
    content_topic: String,
    timestamp: u64,
}

#[derive(Deserialize, Debug)]
struct WakuMessageResponse {
    payload: String,
    #[serde(rename = "contentTopic")]
    #[allow(dead_code)]
    content_topic: Option<String>,
}

impl NwakuTransport {
    pub fn new(waku_url: &str) -> Self {
        Self {
            waku_url: waku_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            subscriptions: Mutex::new(HashMap::new()),
        }
    }
}

fn base64_encode(data: &[u8]) -> String {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(alphabet[((triple >> 18) & 0x3F) as usize] as char);
        result.push(alphabet[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(alphabet[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(alphabet[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

fn base64_decode(input: &str) -> Result<Vec<u8>> {
    let input = input.trim_end_matches('=');
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buf = Vec::new();
    let chars: Vec<u8> = input
        .bytes()
        .filter_map(|b| alphabet.iter().position(|&a| a == b).map(|p| p as u8))
        .collect();
    for chunk in chars.chunks(4) {
        let len = chunk.len();
        if len < 2 {
            break;
        }
        let b0 = chunk[0] as u32;
        let b1 = chunk[1] as u32;
        let b2 = if len > 2 { chunk[2] as u32 } else { 0 };
        let b3 = if len > 3 { chunk[3] as u32 } else { 0 };
        let triple = (b0 << 18) | (b1 << 12) | (b2 << 6) | b3;
        buf.push(((triple >> 16) & 0xFF) as u8);
        if len > 2 {
            buf.push(((triple >> 8) & 0xFF) as u8);
        }
        if len > 3 {
            buf.push((triple & 0xFF) as u8);
        }
    }
    Ok(buf)
}

/// URL-encode a content topic for nwaku REST API.
fn encode_topic(topic: &str) -> String {
    topic.replace('/', "%2F")
}

const DEFAULT_PUBSUB_TOPIC: &str = "/waku/2/default-waku/proto";

/// Poll nwaku REST API for messages on a specific content topic.
async fn poll_rest_messages(
    client: &reqwest::Client,
    waku_url: &str,
    topic: &str,
) -> Result<Vec<Vec<u8>>> {
    let url = format!(
        "{}/relay/v1/messages/{}",
        waku_url,
        encode_topic(DEFAULT_PUBSUB_TOPIC)
    );

    let resp = client
        .get(&url)
        .send()
        .await
        .context("Failed to poll nwaku")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("nwaku poll failed ({}): {}", status, body);
    }

    let messages: Vec<WakuMessageResponse> = resp.json().await.unwrap_or_default();

    let mut payloads = Vec::new();
    for msg in messages {
        if let Some(ct) = &msg.content_topic {
            if ct != topic {
                continue;
            }
        }
        if let Ok(decoded) = base64_decode(&msg.payload) {
            payloads.push(decoded);
        }
    }
    Ok(payloads)
}

#[async_trait]
impl Transport for NwakuTransport {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let url = format!(
            "{}/relay/v1/messages/{}",
            self.waku_url,
            encode_topic(DEFAULT_PUBSUB_TOPIC)
        );

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let msg = RelayMessage {
            payload: base64_encode(payload),
            content_topic: topic.to_string(),
            timestamp: now,
        };

        let resp = self
            .client
            .post(&url)
            .json(&msg)
            .send()
            .await
            .context("Failed to publish to nwaku")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("nwaku publish failed ({}): {}", status, body);
        }
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        // Clean up existing subscription for this topic
        if let Some(handle) = self.subscriptions.lock().unwrap().remove(topic) {
            handle.abort();
        }

        let (tx, rx) = mpsc::channel(256);
        let client = self.client.clone();
        let waku_url = self.waku_url.clone();
        let topic_owned = topic.to_string();

        let handle = tokio::spawn(async move {
            use std::collections::HashSet;
            use std::hash::{Hash, Hasher};
            let mut seen: HashSet<u64> = HashSet::new();

            loop {
                if let Ok(messages) = poll_rest_messages(&client, &waku_url, &topic_owned).await {
                    for msg in messages {
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        msg.hash(&mut hasher);
                        let h = hasher.finish();
                        if seen.insert(h) && tx.send(msg).await.is_err() {
                            return; // Receiver dropped
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });

        self.subscriptions
            .lock()
            .unwrap()
            .insert(topic.to_string(), handle);
        Ok(rx)
    }

    async fn unsubscribe(&self, topic: &str) -> Result<()> {
        if let Some(handle) = self.subscriptions.lock().unwrap().remove(topic) {
            handle.abort();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_roundtrip() {
        let data = b"Hello, Waku!";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_base64_empty() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_base64_padding() {
        // 1 byte -> 4 chars with ==
        let encoded = base64_encode(b"a");
        assert!(encoded.ends_with("=="));
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, b"a");

        // 2 bytes -> 4 chars with =
        let encoded = base64_encode(b"ab");
        assert!(encoded.ends_with('=') && !encoded.ends_with("=="));
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, b"ab");
    }

    #[test]
    fn test_encode_topic() {
        assert_eq!(
            encode_topic("/waku/2/default-waku/proto"),
            "%2Fwaku%2F2%2Fdefault-waku%2Fproto"
        );
    }

    #[test]
    fn test_transport_creation() {
        let t = NwakuTransport::new("http://localhost:8645");
        assert_eq!(t.waku_url, "http://localhost:8645");
    }
}
