//! Integration test: presence announcements and PeerMap discovery.

use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::memory::InMemoryTransport;

#[tokio::test]
async fn test_presence_announce_and_discover() {
    let transport = InMemoryTransport::new();

    // Two agents on the same transport
    let alice = WakuA2ANode::new(
        "alice",
        "Alice agent",
        vec!["summarize".into(), "text".into()],
        transport.clone(),
    );
    let bob = WakuA2ANode::new(
        "bob",
        "Bob agent",
        vec!["translate".into(), "text".into()],
        transport.clone(),
    );

    // Both subscribe to presence before announcements
    alice.poll_presence().await.unwrap();
    bob.poll_presence().await.unwrap();

    // Alice announces
    alice.announce_presence().await.unwrap();
    // Bob polls and sees Alice
    let count = bob.poll_presence().await.unwrap();
    assert_eq!(count, 1);
    let alice_info = bob.peers().get(alice.pubkey()).unwrap();
    assert_eq!(alice_info.name, "alice");
    assert!(alice_info.capabilities.contains(&"summarize".to_string()));

    // Bob announces
    bob.announce_presence().await.unwrap();
    // Alice polls and sees Bob
    let count = alice.poll_presence().await.unwrap();
    assert_eq!(count, 1);
    let bob_info = alice.peers().get(bob.pubkey()).unwrap();
    assert_eq!(bob_info.name, "bob");
    assert!(bob_info.capabilities.contains(&"translate".to_string()));
}

#[tokio::test]
async fn test_find_peers_by_capability() {
    let transport = InMemoryTransport::new();

    let summarizer = WakuA2ANode::new("summarizer", "Summarizer", vec!["summarize".into()], transport.clone());
    let translator = WakuA2ANode::new("translator", "Translator", vec!["translate".into()], transport.clone());
    let polyglot = WakuA2ANode::new("polyglot", "Polyglot", vec!["summarize".into(), "translate".into()], transport.clone());

    let observer = WakuA2ANode::new("observer", "Observer", vec![], transport.clone());
    observer.poll_presence().await.unwrap();

    // All announce
    summarizer.announce_presence().await.unwrap();
    translator.announce_presence().await.unwrap();
    polyglot.announce_presence().await.unwrap();

    // Observer polls
    let count = observer.poll_presence().await.unwrap();
    assert_eq!(count, 3);

    // Query by capability
    let summarizers = observer.find_peers_by_capability("summarize");
    assert_eq!(summarizers.len(), 2); // summarizer + polyglot

    let translators = observer.find_peers_by_capability("translate");
    assert_eq!(translators.len(), 2); // translator + polyglot

    let coders = observer.find_peers_by_capability("code");
    assert!(coders.is_empty());
}

#[tokio::test]
async fn test_self_announcements_ignored() {
    let transport = InMemoryTransport::new();
    let node = WakuA2ANode::new("solo", "Solo agent", vec!["text".into()], transport);

    node.poll_presence().await.unwrap();
    node.announce_presence().await.unwrap();
    let count = node.poll_presence().await.unwrap();
    assert_eq!(count, 0); // should not see own announcement
    assert!(node.peers().is_empty());
}
