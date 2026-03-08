//! Waku presence broadcasts and live peer discovery.
//!
//! Agents announce themselves on `/lmao/1/presence/proto`. Other agents
//! listen, build a [`PeerMap`], and query it by capability when they need
//! to route a task to an agent they haven't talked to before.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use logos_messaging_a2a_core::PresenceAnnouncement;

/// Information about a live peer, derived from its presence announcement.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerInfo {
    /// Human-readable name.
    pub name: String,
    /// Capabilities advertised by this peer.
    pub capabilities: Vec<String>,
    /// Waku content topic where the peer receives tasks.
    pub waku_topic: String,
    /// TTL in seconds — the peer promises to re-announce before this expires.
    pub ttl_secs: u64,
    /// Unix timestamp (seconds) when we last saw an announcement from this peer.
    pub last_seen: u64,
}

impl PeerInfo {
    /// Whether this entry has expired (no refresh within TTL).
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.ttl_secs == 0 || now.saturating_sub(self.last_seen) > self.ttl_secs
    }

    /// Whether this entry is expired relative to a given timestamp.
    pub fn is_expired_at(&self, now_secs: u64) -> bool {
        self.ttl_secs == 0 || now_secs.saturating_sub(self.last_seen) > self.ttl_secs
    }
}

/// Thread-safe map of live peers, keyed by `agent_id` (public key hex).
///
/// Automatically updated when presence announcements are received.
/// Stale entries (past TTL) are lazily evicted on query.
#[derive(Debug)]
pub struct PeerMap {
    peers: Mutex<HashMap<String, PeerInfo>>,
}

impl PeerMap {
    /// Create an empty peer map.
    pub fn new() -> Self {
        Self {
            peers: Mutex::new(HashMap::new()),
        }
    }

    /// Update (or insert) a peer from a presence announcement.
    pub fn update(&self, announcement: &PresenceAnnouncement) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let info = PeerInfo {
            name: announcement.name.clone(),
            capabilities: announcement.capabilities.clone(),
            waku_topic: announcement.waku_topic.clone(),
            ttl_secs: announcement.ttl_secs,
            last_seen: now,
        };

        self.peers
            .lock()
            .unwrap()
            .insert(announcement.agent_id.clone(), info);
    }

    /// Get info for a specific agent (returns `None` if unknown or expired).
    pub fn get(&self, agent_id: &str) -> Option<PeerInfo> {
        let peers = self.peers.lock().unwrap();
        peers.get(agent_id).and_then(|info| {
            if info.is_expired() {
                None
            } else {
                Some(info.clone())
            }
        })
    }

    /// Find all live peers that advertise a given capability.
    pub fn find_by_capability(&self, capability: &str) -> Vec<(String, PeerInfo)> {
        let peers = self.peers.lock().unwrap();
        peers
            .iter()
            .filter(|(_, info)| {
                !info.is_expired() && info.capabilities.contains(&capability.to_string())
            })
            .map(|(id, info)| (id.clone(), info.clone()))
            .collect()
    }

    /// Return all live (non-expired) peers.
    pub fn all_live(&self) -> Vec<(String, PeerInfo)> {
        let peers = self.peers.lock().unwrap();
        peers
            .iter()
            .filter(|(_, info)| !info.is_expired())
            .map(|(id, info)| (id.clone(), info.clone()))
            .collect()
    }

    /// Number of peers (including potentially expired ones).
    pub fn len(&self) -> usize {
        self.peers.lock().unwrap().len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.peers.lock().unwrap().is_empty()
    }

    /// Remove expired entries. Returns the number removed.
    pub fn evict_expired(&self) -> usize {
        let mut peers = self.peers.lock().unwrap();
        let before = peers.len();
        peers.retain(|_, info| !info.is_expired());
        before - peers.len()
    }
}

impl Default for PeerMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_announcement(
        agent_id: &str,
        name: &str,
        caps: Vec<&str>,
        ttl: u64,
    ) -> PresenceAnnouncement {
        PresenceAnnouncement {
            agent_id: agent_id.to_string(),
            name: name.to_string(),
            capabilities: caps.into_iter().map(String::from).collect(),
            waku_topic: format!("/waku-a2a/1/task/{}/proto", agent_id),
            ttl_secs: ttl,
            signature: None,
        }
    }

    #[test]
    fn test_peer_map_insert_and_get() {
        let map = PeerMap::new();
        assert!(map.is_empty());

        let ann = make_announcement("02aa", "echo", vec!["text"], 300);
        map.update(&ann);

        assert_eq!(map.len(), 1);
        let info = map.get("02aa").unwrap();
        assert_eq!(info.name, "echo");
        assert_eq!(info.capabilities, vec!["text"]);
    }

    #[test]
    fn test_peer_map_update_refreshes() {
        let map = PeerMap::new();
        let ann1 = make_announcement("02aa", "echo-v1", vec!["text"], 300);
        map.update(&ann1);
        assert_eq!(map.get("02aa").unwrap().name, "echo-v1");

        let ann2 = make_announcement("02aa", "echo-v2", vec!["text", "summarize"], 600);
        map.update(&ann2);
        let info = map.get("02aa").unwrap();
        assert_eq!(info.name, "echo-v2");
        assert_eq!(info.capabilities, vec!["text", "summarize"]);
        assert_eq!(info.ttl_secs, 600);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_find_by_capability() {
        let map = PeerMap::new();
        map.update(&make_announcement(
            "02aa",
            "summarizer",
            vec!["summarize", "text"],
            300,
        ));
        map.update(&make_announcement(
            "02bb",
            "translator",
            vec!["translate", "text"],
            300,
        ));
        map.update(&make_announcement("02cc", "coder", vec!["code"], 300));

        let text_peers = map.find_by_capability("text");
        assert_eq!(text_peers.len(), 2);

        let code_peers = map.find_by_capability("code");
        assert_eq!(code_peers.len(), 1);
        assert_eq!(code_peers[0].0, "02cc");

        assert!(map.find_by_capability("nonexistent").is_empty());
    }

    #[test]
    fn test_expired_peer_not_returned() {
        let map = PeerMap::new();
        let ann = make_announcement("02aa", "expired", vec!["text"], 0);
        map.update(&ann);

        assert_eq!(map.len(), 1);
        assert!(map.get("02aa").is_none());
        assert!(map.find_by_capability("text").is_empty());
        assert!(map.all_live().is_empty());
    }

    #[test]
    fn test_evict_expired() {
        let map = PeerMap::new();
        map.update(&make_announcement("02aa", "alive", vec!["text"], 9999));
        map.update(&make_announcement("02bb", "dead", vec!["text"], 0));

        assert_eq!(map.len(), 2);
        let evicted = map.evict_expired();
        assert_eq!(evicted, 1);
        assert_eq!(map.len(), 1);
        assert!(map.get("02aa").is_some());
    }

    #[test]
    fn test_all_live() {
        let map = PeerMap::new();
        map.update(&make_announcement("02aa", "a", vec!["text"], 9999));
        map.update(&make_announcement("02bb", "b", vec!["code"], 9999));

        let live = map.all_live();
        assert_eq!(live.len(), 2);
    }

    #[test]
    fn test_peer_info_is_expired_at() {
        let info = PeerInfo {
            name: "test".to_string(),
            capabilities: vec![],
            waku_topic: "".to_string(),
            ttl_secs: 300,
            last_seen: 1000,
        };
        assert!(!info.is_expired_at(1100));
        assert!(!info.is_expired_at(1300));
        assert!(info.is_expired_at(1301));
    }

    // --- Additional edge case coverage ---

    #[test]
    fn test_peer_map_get_nonexistent_returns_none() {
        let map = PeerMap::new();
        assert!(map.get("nonexistent").is_none());
    }

    #[test]
    fn test_peer_map_find_by_capability_empty_map() {
        let map = PeerMap::new();
        assert!(map.find_by_capability("anything").is_empty());
    }

    #[test]
    fn test_peer_map_all_live_empty_map() {
        let map = PeerMap::new();
        assert!(map.all_live().is_empty());
    }

    #[test]
    fn test_evict_expired_on_empty_map() {
        let map = PeerMap::new();
        assert_eq!(map.evict_expired(), 0);
    }

    #[test]
    fn test_evict_expired_all_alive() {
        let map = PeerMap::new();
        map.update(&make_announcement("a", "a", vec!["x"], 9999));
        map.update(&make_announcement("b", "b", vec!["y"], 9999));
        assert_eq!(map.evict_expired(), 0);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_evict_expired_all_dead() {
        let map = PeerMap::new();
        map.update(&make_announcement("a", "a", vec!["x"], 0));
        map.update(&make_announcement("b", "b", vec!["y"], 0));
        assert_eq!(map.evict_expired(), 2);
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn test_peer_info_is_expired_at_boundary() {
        let info = PeerInfo {
            name: "test".to_string(),
            capabilities: vec![],
            waku_topic: "".to_string(),
            ttl_secs: 300,
            last_seen: 1000,
        };
        // Exactly at TTL boundary: 1000 + 300 = 1300, elapsed = 300, not > 300
        assert!(!info.is_expired_at(1300));
        // One second past: elapsed = 301 > 300
        assert!(info.is_expired_at(1301));
    }

    #[test]
    fn test_peer_info_zero_ttl_always_expired() {
        let info = PeerInfo {
            name: "test".to_string(),
            capabilities: vec![],
            waku_topic: "".to_string(),
            ttl_secs: 0,
            last_seen: 1000,
        };
        assert!(info.is_expired_at(1000));
        assert!(info.is_expired_at(0));
    }

    #[test]
    fn test_peer_info_is_expired_at_saturating_sub() {
        // now_secs < last_seen — should not panic or wrap
        let info = PeerInfo {
            name: "test".to_string(),
            capabilities: vec![],
            waku_topic: "".to_string(),
            ttl_secs: 300,
            last_seen: 1000,
        };
        // now_secs = 500 < last_seen = 1000, saturating_sub gives 0, 0 <= 300
        assert!(!info.is_expired_at(500));
    }

    #[test]
    fn test_update_same_peer_overwrites() {
        let map = PeerMap::new();
        map.update(&make_announcement("peer1", "name-v1", vec!["cap1"], 300));
        assert_eq!(map.get("peer1").unwrap().name, "name-v1");

        map.update(&make_announcement(
            "peer1",
            "name-v2",
            vec!["cap1", "cap2"],
            600,
        ));
        let info = map.get("peer1").unwrap();
        assert_eq!(info.name, "name-v2");
        assert_eq!(info.capabilities, vec!["cap1", "cap2"]);
        assert_eq!(info.ttl_secs, 600);
        assert_eq!(map.len(), 1); // still one entry
    }

    #[test]
    fn test_find_by_capability_with_empty_capability_string() {
        let map = PeerMap::new();
        map.update(&make_announcement("a", "a", vec![""], 9999));
        // Should find it when searching for empty string
        assert_eq!(map.find_by_capability("").len(), 1);
        assert!(map.find_by_capability("text").is_empty());
    }

    #[test]
    fn test_peer_map_many_peers() {
        let map = PeerMap::new();
        for i in 0..100 {
            map.update(&make_announcement(
                &format!("peer{i}"),
                &format!("agent-{i}"),
                vec!["common"],
                9999,
            ));
        }
        assert_eq!(map.len(), 100);
        assert_eq!(map.all_live().len(), 100);
        assert_eq!(map.find_by_capability("common").len(), 100);
    }
}
