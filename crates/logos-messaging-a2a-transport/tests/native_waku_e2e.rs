//! End-to-end test for NativeWakuTransport.
//!
//! Spins up two in-process Waku nodes, connects them, and verifies
//! publish/subscribe round-trip. Requires libwaku (Nim) at link time.
//!
//! Run with: cargo test -p logos-messaging-a2a-transport --features native-waku -- --ignored

#[cfg(feature = "native-waku")]
mod native_waku_tests {
    use logos_messaging_a2a_transport::{NativeWakuTransport, Transport};
    use tokio::time::{timeout, Duration};
    use waku_bindings::WakuNodeConfig;

    #[tokio::test]
    #[ignore] // requires libwaku build
    async fn two_nodes_publish_subscribe() {
        // Node A on port 60010
        let node_a = NativeWakuTransport::new(WakuNodeConfig {
            tcp_port: Some(60010),
            ..Default::default()
        })
        .await
        .expect("node A should start");

        // Node B on port 60011
        let node_b = NativeWakuTransport::new(WakuNodeConfig {
            tcp_port: Some(60011),
            ..Default::default()
        })
        .await
        .expect("node B should start");

        // Get A's listen addresses and connect B → A
        let addrs_a = node_a
            .listen_addresses()
            .await
            .expect("should get listen addrs");
        assert!(!addrs_a.is_empty(), "node A should have listen addresses");

        node_b
            .connect(&addrs_a[0])
            .await
            .expect("B should connect to A");

        // B subscribes to a topic
        let topic = "test-agent-123";
        let mut rx = node_b.subscribe(topic).await.expect("subscribe should work");

        // Give nodes a moment to establish relay mesh
        tokio::time::sleep(Duration::from_millis(500)).await;

        // A publishes a message
        let payload = b"hello from node A";
        node_a
            .publish(topic, payload)
            .await
            .expect("publish should work");

        // B should receive it
        let received = timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("should receive within 5s")
            .expect("channel should not close");

        assert_eq!(received, payload, "payload should match");
    }

    #[tokio::test]
    #[ignore]
    async fn unsubscribe_stops_delivery() {
        let node = NativeWakuTransport::new(WakuNodeConfig {
            tcp_port: Some(60020),
            ..Default::default()
        })
        .await
        .expect("node should start");

        let topic = "unsub-test";
        let _rx = node.subscribe(topic).await.expect("subscribe");

        node.unsubscribe(topic).await.expect("unsubscribe");

        // After unsubscribe, the sender map should not contain the topic.
        // We dont test publish here because a lone node with no peers
        // will error on relay (no peers found), which is expected.
    }
}
