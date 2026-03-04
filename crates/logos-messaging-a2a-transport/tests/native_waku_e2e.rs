//! E2E integration test for NativeWakuTransport.
//!
//! Requires libwaku (Nim) build — run with:
//!   cargo test -p logos-messaging-a2a-transport --features native-waku --test native_waku_e2e -- --ignored

#[cfg(feature = "native-waku")]
mod tests {
    use logos_messaging_a2a_transport::{NativeWakuTransport, Transport};
    use waku_bindings::WakuNodeConfig;

    /// Spin up two in-process waku nodes, connect them, publish a message from
    /// node A and verify node B receives it.
    #[tokio::test]
    #[ignore] // requires libwaku build
    async fn e2e_two_nodes_native_transport() {
        let config_a = WakuNodeConfig {
            tcp_port: Some(60010),
            ..Default::default()
        };
        let config_b = WakuNodeConfig {
            tcp_port: Some(60011),
            ..Default::default()
        };

        let node_a = NativeWakuTransport::new(config_a)
            .await
            .expect("node A start");
        let node_b = NativeWakuTransport::new(config_b)
            .await
            .expect("node B start");

        // Connect B to A
        let addrs_a = node_a.listen_addresses().await.expect("listen addrs");
        assert!(!addrs_a.is_empty(), "node A should have listen addresses");
        node_b
            .connect(&addrs_a[0])
            .await
            .expect("B connects to A");

        // B subscribes to topic
        let topic = "test-agent-1";
        let mut rx = node_b.subscribe(topic).await.expect("subscribe");

        // Give relay mesh time to form
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // A publishes
        let payload = b"hello from node A";
        node_a
            .publish(topic, payload)
            .await
            .expect("publish");

        // B should receive
        let received = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed");

        assert_eq!(received, payload);
    }
}
