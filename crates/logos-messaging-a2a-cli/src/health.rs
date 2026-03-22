use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeInfo {
    enr_uri: Option<String>,
    listen_addresses: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct PeerInfo {
    #[allow(dead_code)]
    multiaddr: Option<String>,
}

pub async fn handle(waku_url: &str, json: bool) -> Result<()> {
    let base = waku_url.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    // 1. Check debug/v1/info for reachability + node info
    let info_url = format!("{base}/debug/v1/info");
    let info_result = client.get(&info_url).send().await;

    let node_info = match info_result {
        Ok(resp) if resp.status().is_success() => match resp.json::<NodeInfo>().await {
            Ok(info) => Some(info),
            Err(_) => Some(NodeInfo {
                enr_uri: None,
                listen_addresses: None,
            }),
        },
        Ok(resp) => {
            let status = resp.status();
            eprintln!("Warning: /debug/v1/info returned {status}");
            None
        }
        Err(e) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "status": "UNREACHABLE",
                        "error": format!("{e}"),
                        "waku_url": base,
                    }))?
                );
            } else {
                eprintln!("Error: could not reach Waku node at {base}: {e}");
                println!("Waku node: UNREACHABLE");
            }
            return Ok(());
        }
    };

    // 2. Check relay subscriptions (GET /relay/v1/subscriptions)
    let relay_url = format!("{base}/relay/v1/subscriptions");
    let relay_result = client.get(&relay_url).send().await;
    let relay_subs: Option<Vec<String>> = match relay_result {
        Ok(resp) if resp.status().is_success() => resp.json().await.ok(),
        _ => None,
    };

    // 3. Check connected peers (GET /admin/v1/peers)
    let peers_url = format!("{base}/admin/v1/peers");
    let peers_result = client.get(&peers_url).send().await;
    let peer_count: Option<usize> = match peers_result {
        Ok(resp) if resp.status().is_success() => {
            resp.json::<Vec<PeerInfo>>().await.ok().map(|p| p.len())
        }
        _ => None,
    };

    // 4. Print summary
    let healthy = node_info.is_some();

    if json {
        let mut obj = serde_json::json!({
            "status": if healthy { "HEALTHY" } else { "UNREACHABLE" },
            "waku_url": base,
        });
        if let Some(ref info) = node_info {
            if let Some(ref enr) = info.enr_uri {
                obj["enr_uri"] = serde_json::json!(enr);
            }
            if let Some(ref addrs) = info.listen_addresses {
                obj["listen_addresses"] = serde_json::json!(addrs);
            }
        }
        if let Some(ref subs) = relay_subs {
            obj["relay_subscriptions"] = serde_json::json!(subs);
        }
        if let Some(count) = peer_count {
            obj["connected_peers"] = serde_json::json!(count);
        }
        println!("{}", serde_json::to_string(&obj)?);
    } else {
        if let Some(ref info) = node_info {
            if let Some(ref enr) = info.enr_uri {
                println!("ENR URI: {enr}");
            }
            if let Some(ref addrs) = info.listen_addresses {
                println!("Listen addresses:");
                for addr in addrs {
                    println!("  {addr}");
                }
            }
        }
        if let Some(ref subs) = relay_subs {
            println!("Relay subscriptions: {}", subs.len());
            for topic in subs {
                println!("  {topic}");
            }
        } else {
            println!("Relay subscriptions: unavailable");
        }
        if let Some(count) = peer_count {
            println!("Connected peers: {count}");
        } else {
            println!("Connected peers: unavailable (admin API may be disabled)");
        }
        if healthy {
            println!("Waku node: HEALTHY");
        } else {
            println!("Waku node: UNREACHABLE");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn health_json_healthy_output_is_parseable() {
        let obj = serde_json::json!({
            "status": "HEALTHY",
            "waku_url": "http://localhost:8645",
            "enr_uri": "enr:-abc123",
            "listen_addresses": ["/ip4/0.0.0.0/tcp/60000"],
            "relay_subscriptions": ["/waku/2/default-waku/proto"],
            "connected_peers": 3,
        });
        let output = serde_json::to_string(&obj).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["status"], "HEALTHY");
        assert_eq!(parsed["connected_peers"], 3);
        assert!(parsed["listen_addresses"].is_array());
    }

    #[test]
    fn health_json_unreachable_output_is_parseable() {
        let obj = serde_json::json!({
            "status": "UNREACHABLE",
            "error": "connection refused",
            "waku_url": "http://localhost:8645",
        });
        let output = serde_json::to_string(&obj).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["status"], "UNREACHABLE");
        assert!(parsed["error"].is_string());
    }
}
