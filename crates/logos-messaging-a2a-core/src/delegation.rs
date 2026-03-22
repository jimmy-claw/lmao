use serde::{Deserialize, Serialize};

/// Strategy for selecting which peer(s) to delegate a subtask to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DelegationStrategy {
    /// Pick the first available peer (any capability).
    FirstAvailable,
    /// Broadcast to all matching peers and collect responses.
    BroadcastCollect,
    /// Distribute subtasks evenly across peers using round-robin rotation.
    RoundRobin,
    /// Pick a peer that advertises a specific capability.
    CapabilityMatch {
        /// The required capability string.
        capability: String,
    },
}

/// A request to delegate a subtask to one or more peers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegationRequest {
    /// Identifier of the parent task that spawned this subtask.
    pub parent_task_id: String,
    /// Text content of the subtask to be delegated.
    pub subtask_text: String,
    /// Strategy for selecting the delegate(s).
    pub strategy: DelegationStrategy,
    /// How long to wait (seconds) for the delegate to respond.
    /// `0` means use the default timeout.
    pub timeout_secs: u64,
}

/// The result of a delegated subtask, including which agent handled it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegationResult {
    /// ID of the parent task.
    pub parent_task_id: String,
    /// ID of the subtask that was delegated.
    pub subtask_id: String,
    /// Public key (hex) of the agent that processed the subtask.
    pub agent_id: String,
    /// The text result returned by the delegate agent, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_text: Option<String>,
    /// Whether the delegation succeeded.
    pub success: bool,
    /// Error message if the delegation failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delegation_strategy_first_available_serialization() {
        let strategy = DelegationStrategy::FirstAvailable;
        let json = serde_json::to_string(&strategy).unwrap();
        assert!(json.contains("\"type\":\"first_available\""));
        let deserialized: DelegationStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(strategy, deserialized);
    }

    #[test]
    fn test_delegation_strategy_broadcast_collect_serialization() {
        let strategy = DelegationStrategy::BroadcastCollect;
        let json = serde_json::to_string(&strategy).unwrap();
        assert!(json.contains("\"type\":\"broadcast_collect\""));
        let deserialized: DelegationStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(strategy, deserialized);
    }

    #[test]
    fn test_delegation_strategy_round_robin_serialization() {
        let strategy = DelegationStrategy::RoundRobin;
        let json = serde_json::to_string(&strategy).unwrap();
        assert!(json.contains("\"type\":\"round_robin\""));
        let deserialized: DelegationStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(strategy, deserialized);
    }

    #[test]
    fn test_delegation_strategy_capability_match_serialization() {
        let strategy = DelegationStrategy::CapabilityMatch {
            capability: "summarize".to_string(),
        };
        let json = serde_json::to_string(&strategy).unwrap();
        assert!(json.contains("\"type\":\"capability_match\""));
        assert!(json.contains("\"capability\":\"summarize\""));
        let deserialized: DelegationStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(strategy, deserialized);
    }

    #[test]
    fn test_delegation_request_serialization() {
        let req = DelegationRequest {
            parent_task_id: "parent-123".to_string(),
            subtask_text: "Summarize this document".to_string(),
            strategy: DelegationStrategy::CapabilityMatch {
                capability: "summarize".to_string(),
            },
            timeout_secs: 30,
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: DelegationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn test_delegation_result_success_serialization() {
        let result = DelegationResult {
            parent_task_id: "parent-123".to_string(),
            subtask_id: "subtask-456".to_string(),
            agent_id: "02abcdef".to_string(),
            result_text: Some("Summary: ...".to_string()),
            success: true,
            error: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("error"));
        let deserialized: DelegationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deserialized);
    }

    #[test]
    fn test_delegation_result_failure_serialization() {
        let result = DelegationResult {
            parent_task_id: "parent-123".to_string(),
            subtask_id: "subtask-456".to_string(),
            agent_id: "02abcdef".to_string(),
            result_text: None,
            success: false,
            error: Some("timeout".to_string()),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("result_text"));
        assert!(json.contains("\"error\":\"timeout\""));
        let deserialized: DelegationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deserialized);
    }

    #[test]
    fn test_delegation_request_zero_timeout() {
        let req = DelegationRequest {
            parent_task_id: "p".to_string(),
            subtask_text: "task".to_string(),
            strategy: DelegationStrategy::FirstAvailable,
            timeout_secs: 0,
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: DelegationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.timeout_secs, 0);
    }

    #[test]
    fn test_delegation_strategy_clone_and_debug() {
        let strategy = DelegationStrategy::BroadcastCollect;
        let cloned = strategy.clone();
        assert_eq!(strategy, cloned);
        let debug = format!("{:?}", strategy);
        assert!(debug.contains("BroadcastCollect"));
    }

    #[test]
    fn test_delegation_request_clone_and_debug() {
        let req = DelegationRequest {
            parent_task_id: "p".to_string(),
            subtask_text: "t".to_string(),
            strategy: DelegationStrategy::FirstAvailable,
            timeout_secs: 10,
        };
        let cloned = req.clone();
        assert_eq!(req, cloned);
        let debug = format!("{:?}", req);
        assert!(debug.contains("DelegationRequest"));
    }

    #[test]
    fn test_delegation_result_clone_and_debug() {
        let result = DelegationResult {
            parent_task_id: "p".to_string(),
            subtask_id: "s".to_string(),
            agent_id: "02ab".to_string(),
            result_text: Some("ok".to_string()),
            success: true,
            error: None,
        };
        let cloned = result.clone();
        assert_eq!(result, cloned);
        let debug = format!("{:?}", result);
        assert!(debug.contains("DelegationResult"));
    }

    #[test]
    fn test_delegation_strategy_all_variants_distinct_type_tags() {
        let variants: Vec<DelegationStrategy> = vec![
            DelegationStrategy::FirstAvailable,
            DelegationStrategy::BroadcastCollect,
            DelegationStrategy::RoundRobin,
            DelegationStrategy::CapabilityMatch {
                capability: "x".to_string(),
            },
        ];
        let mut tags: Vec<String> = Vec::new();
        for v in variants {
            let json = serde_json::to_string(&v).unwrap();
            let val: serde_json::Value = serde_json::from_str(&json).unwrap();
            tags.push(val["type"].as_str().unwrap().to_string());
        }
        let unique: std::collections::HashSet<&String> = tags.iter().collect();
        assert_eq!(unique.len(), tags.len());
    }

    #[test]
    fn test_delegation_strategy_invalid_type_tag_fails() {
        let json = r#"{"type":"nonexistent"}"#;
        let result = serde_json::from_str::<DelegationStrategy>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_delegation_request_missing_fields_fails() {
        let json = r#"{"parent_task_id":"p"}"#;
        let result = serde_json::from_str::<DelegationRequest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_delegation_result_backward_compat_without_optional_fields() {
        let json = r#"{"parent_task_id":"p","subtask_id":"s","agent_id":"02ab","success":true}"#;
        let result: DelegationResult = serde_json::from_str(json).unwrap();
        assert!(result.result_text.is_none());
        assert!(result.error.is_none());
        assert!(result.success);
    }
}
