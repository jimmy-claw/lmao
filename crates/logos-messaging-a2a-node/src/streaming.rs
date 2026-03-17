//! Streaming operations for [`WakuA2ANode`](crate::WakuA2ANode).

use logos_messaging_a2a_core::{topics, A2AEnvelope, Task, TaskStreamChunk};
use logos_messaging_a2a_transport::Transport;

use crate::metrics::Metrics;
use crate::{Result, WakuA2ANode};

impl<T: Transport> WakuA2ANode<T> {
    /// Publish a sequence of stream chunks for a task.
    ///
    /// Each string in `chunks` becomes a `TaskStreamChunk` with incrementing
    /// `chunk_index`. The last chunk is automatically marked `is_final = true`.
    /// Chunks are published to a dedicated stream topic derived from the task ID.
    pub async fn respond_stream(&self, task: &Task, chunks: Vec<String>) -> Result<()> {
        let topic = topics::stream_topic(&task.id);
        let total = chunks.len();
        for (i, text) in chunks.into_iter().enumerate() {
            let chunk = TaskStreamChunk {
                task_id: task.id.clone(),
                chunk_index: i as u32,
                text,
                is_final: i == total - 1,
            };
            let envelope = A2AEnvelope::StreamChunk(chunk);
            let payload = serde_json::to_vec(&envelope)?;
            self.channel.transport().publish(&topic, &payload).await?;
        }
        Metrics::inc_by(&self.metrics.stream_chunks_sent, total as u64);
        Metrics::inc_by(&self.metrics.messages_published, total as u64);
        tracing::info!(task_id = %task.id, chunks = total, "Streamed chunks for task");
        Ok(())
    }

    /// Poll for stream chunks for a given task ID.
    ///
    /// Subscribes to the task's stream topic, drains available chunks,
    /// buffers them internally, and returns all chunks received so far
    /// sorted by `chunk_index`.
    pub async fn poll_stream_chunks(&self, task_id: &str) -> Result<Vec<TaskStreamChunk>> {
        let topic = topics::stream_topic(task_id);
        let mut rx = self.channel.transport().subscribe(&topic).await?;

        // Drain all available messages from the subscription
        let mut new_chunks = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Ok(A2AEnvelope::StreamChunk(chunk)) = serde_json::from_slice::<A2AEnvelope>(&msg)
            {
                if chunk.task_id == task_id {
                    new_chunks.push(chunk);
                }
            }
        }

        let _ = self.channel.transport().unsubscribe(&topic).await;

        Metrics::inc_by(
            &self.metrics.stream_chunks_received,
            new_chunks.len() as u64,
        );

        // Merge into the internal buffer
        let mut buffer = self.stream_chunks.lock().unwrap();
        let entry = buffer.entry(task_id.to_string()).or_default();
        for chunk in new_chunks {
            // Avoid duplicates by chunk_index
            if !entry.iter().any(|c| c.chunk_index == chunk.chunk_index) {
                entry.push(chunk);
            }
        }
        entry.sort_by_key(|c| c.chunk_index);
        Ok(entry.clone())
    }

    /// Reassemble all buffered stream chunks for a task into a single string.
    ///
    /// Returns `None` if no chunks are buffered or the stream is incomplete
    /// (no final chunk received yet).
    pub fn reassemble_stream(&self, task_id: &str) -> Option<String> {
        let buffer = self.stream_chunks.lock().unwrap();
        let chunks = buffer.get(task_id)?;
        if chunks.is_empty() {
            return None;
        }
        if !chunks.iter().any(|c| c.is_final) {
            return None;
        }
        Some(chunks.iter().map(|c| c.text.as_str()).collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::WakuA2ANode;
    use logos_messaging_a2a_core::{topics, A2AEnvelope, Task, TaskStreamChunk};
    use logos_messaging_a2a_transport::memory::InMemoryTransport;
    use logos_messaging_a2a_transport::Transport;

    fn make_node_with_transport(
        name: &str,
        transport: InMemoryTransport,
    ) -> WakuA2ANode<InMemoryTransport> {
        WakuA2ANode::new(
            name,
            &format!("{} agent", name),
            vec!["text".into()],
            transport,
        )
    }

    #[tokio::test]
    async fn respond_stream_publishes_chunks() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport.clone());
        let task = Task::new(node.pubkey(), "02recipient", "do something");

        let chunks = vec!["Hello ".to_string(), "world".to_string(), "!".to_string()];
        node.respond_stream(&task, chunks).await.unwrap();

        // Subscribe to stream topic — history replay gives us published chunks
        let stream_topic = topics::stream_topic(&task.id);
        let mut rx = transport.subscribe(&stream_topic).await.unwrap();

        for i in 0..3 {
            let msg = rx.try_recv().unwrap();
            let envelope: A2AEnvelope = serde_json::from_slice(&msg).unwrap();
            match envelope {
                A2AEnvelope::StreamChunk(chunk) => {
                    assert_eq!(chunk.task_id, task.id);
                    assert_eq!(chunk.chunk_index, i as u32);
                    assert_eq!(chunk.is_final, i == 2);
                }
                _ => panic!("Expected StreamChunk envelope"),
            }
        }
        // No more messages
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn poll_stream_chunks_receives_ordered() {
        let transport = InMemoryTransport::new();
        let task_id = "test-task-123";
        let stream_topic = topics::stream_topic(task_id);

        // Inject chunks out of order
        for (idx, text, is_final) in [(1, "world", false), (0, "Hello ", false), (2, "!", true)] {
            let chunk = TaskStreamChunk {
                task_id: task_id.to_string(),
                chunk_index: idx,
                text: text.to_string(),
                is_final,
            };
            let envelope = A2AEnvelope::StreamChunk(chunk);
            let payload = serde_json::to_vec(&envelope).unwrap();
            transport.publish(&stream_topic, &payload).await.unwrap();
        }

        let node = make_node_with_transport("receiver", transport);
        let chunks = node.poll_stream_chunks(task_id).await.unwrap();

        assert_eq!(chunks.len(), 3);
        // Should be sorted by chunk_index
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].text, "Hello ");
        assert_eq!(chunks[1].chunk_index, 1);
        assert_eq!(chunks[1].text, "world");
        assert_eq!(chunks[2].chunk_index, 2);
        assert_eq!(chunks[2].text, "!");
        assert!(chunks[2].is_final);
    }

    #[tokio::test]
    async fn poll_stream_deduplicates_chunks() {
        let transport = InMemoryTransport::new();
        let task_id = "dedup-task";
        let stream_topic = topics::stream_topic(task_id);

        // Inject the same chunk twice
        let chunk = TaskStreamChunk {
            task_id: task_id.to_string(),
            chunk_index: 0,
            text: "Hello".to_string(),
            is_final: false,
        };
        let envelope = A2AEnvelope::StreamChunk(chunk);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        let node = make_node_with_transport("receiver", transport);
        let chunks = node.poll_stream_chunks(task_id).await.unwrap();
        assert_eq!(chunks.len(), 1);
    }

    #[tokio::test]
    async fn reassemble_stream_concatenates_text() {
        let transport = InMemoryTransport::new();
        let task_id = "reassemble-task";
        let stream_topic = topics::stream_topic(task_id);

        let texts = ["Hello ", "beautiful ", "world!"];
        for (i, text) in texts.iter().enumerate() {
            let chunk = TaskStreamChunk {
                task_id: task_id.to_string(),
                chunk_index: i as u32,
                text: text.to_string(),
                is_final: i == texts.len() - 1,
            };
            let envelope = A2AEnvelope::StreamChunk(chunk);
            let payload = serde_json::to_vec(&envelope).unwrap();
            transport.publish(&stream_topic, &payload).await.unwrap();
        }

        let node = make_node_with_transport("receiver", transport);
        node.poll_stream_chunks(task_id).await.unwrap();
        let result = node.reassemble_stream(task_id);
        assert_eq!(result, Some("Hello beautiful world!".to_string()));
    }

    #[tokio::test]
    async fn reassemble_returns_none_without_final() {
        let transport = InMemoryTransport::new();
        let task_id = "incomplete-task";
        let stream_topic = topics::stream_topic(task_id);

        let chunk = TaskStreamChunk {
            task_id: task_id.to_string(),
            chunk_index: 0,
            text: "partial".to_string(),
            is_final: false,
        };
        let envelope = A2AEnvelope::StreamChunk(chunk);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        let node = make_node_with_transport("receiver", transport);
        node.poll_stream_chunks(task_id).await.unwrap();
        assert!(node.reassemble_stream(task_id).is_none());
    }

    #[test]
    fn reassemble_returns_none_for_unknown_task() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("receiver", transport);
        assert!(node.reassemble_stream("nonexistent").is_none());
    }

    #[tokio::test]
    async fn respond_stream_single_chunk() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport.clone());
        let task = Task::new(node.pubkey(), "02recipient", "do something");

        // Single chunk should be both first and final
        node.respond_stream(&task, vec!["all at once".to_string()])
            .await
            .unwrap();

        let receiver = make_node_with_transport("receiver", transport);
        let chunks = receiver.poll_stream_chunks(&task.id).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].text, "all at once");
        assert!(chunks[0].is_final);
    }

    #[tokio::test]
    async fn poll_stream_ignores_wrong_task_id() {
        let transport = InMemoryTransport::new();
        let target_task = "target-task";
        let other_task = "other-task";
        let stream_topic = topics::stream_topic(target_task);

        // Inject a chunk with a different task_id on the same topic
        let chunk = TaskStreamChunk {
            task_id: other_task.to_string(),
            chunk_index: 0,
            text: "wrong task".to_string(),
            is_final: true,
        };
        let envelope = A2AEnvelope::StreamChunk(chunk);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        let node = make_node_with_transport("receiver", transport);
        let chunks = node.poll_stream_chunks(target_task).await.unwrap();
        assert!(chunks.is_empty());
    }

    // ── Additional streaming tests ──

    #[tokio::test]
    async fn respond_stream_marks_only_last_chunk_as_final() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport.clone());
        let task = Task::new(node.pubkey(), "02recipient", "do something");

        let chunks = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        node.respond_stream(&task, chunks).await.unwrap();

        let stream_topic = topics::stream_topic(&task.id);
        let mut rx = transport.subscribe(&stream_topic).await.unwrap();

        for i in 0..4 {
            let msg = rx.try_recv().unwrap();
            let envelope: A2AEnvelope = serde_json::from_slice(&msg).unwrap();
            match envelope {
                A2AEnvelope::StreamChunk(chunk) => {
                    assert_eq!(chunk.chunk_index, i as u32);
                    if i == 3 {
                        assert!(chunk.is_final, "last chunk should be final");
                    } else {
                        assert!(!chunk.is_final, "non-last chunk should not be final");
                    }
                }
                _ => panic!("Expected StreamChunk"),
            }
        }
    }

    #[tokio::test]
    async fn respond_stream_increments_metrics() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport);
        let task = Task::new(node.pubkey(), "02recipient", "do something");

        let before = node.metrics();
        assert_eq!(before.stream_chunks_sent, 0);
        assert_eq!(before.messages_published, 0);

        let chunks = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        node.respond_stream(&task, chunks).await.unwrap();

        let after = node.metrics();
        assert_eq!(after.stream_chunks_sent, 3);
        assert_eq!(after.messages_published, 3);
    }

    #[tokio::test]
    async fn poll_stream_chunks_increments_metrics() {
        let transport = InMemoryTransport::new();
        let task_id = "metric-task";
        let stream_topic = topics::stream_topic(task_id);

        let chunk = TaskStreamChunk {
            task_id: task_id.to_string(),
            chunk_index: 0,
            text: "data".to_string(),
            is_final: true,
        };
        let envelope = A2AEnvelope::StreamChunk(chunk);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        let node = make_node_with_transport("receiver", transport);

        let before = node.metrics();
        assert_eq!(before.stream_chunks_received, 0);

        node.poll_stream_chunks(task_id).await.unwrap();

        let after = node.metrics();
        assert_eq!(after.stream_chunks_received, 1);
    }

    #[tokio::test]
    async fn poll_stream_ignores_malformed_messages() {
        let transport = InMemoryTransport::new();
        let task_id = "malformed-stream";
        let stream_topic = topics::stream_topic(task_id);

        // Inject garbage
        transport.publish(&stream_topic, b"not json").await.unwrap();
        transport
            .publish(&stream_topic, b"{\"bad\": true}")
            .await
            .unwrap();

        // Inject a non-StreamChunk envelope
        let ack = A2AEnvelope::Ack {
            message_id: "fake".into(),
        };
        let payload = serde_json::to_vec(&ack).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        let node = make_node_with_transport("receiver", transport);
        let chunks = node.poll_stream_chunks(task_id).await.unwrap();
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn poll_stream_empty_topic_returns_empty() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("receiver", transport);
        let chunks = node.poll_stream_chunks("no-such-task").await.unwrap();
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn respond_stream_and_poll_roundtrip() {
        let transport = InMemoryTransport::new();
        let sender = make_node_with_transport("sender", transport.clone());
        let receiver = make_node_with_transport("receiver", transport);

        let task = Task::new(sender.pubkey(), receiver.pubkey(), "stream me");
        let chunks = vec![
            "Hello ".to_string(),
            "streaming ".to_string(),
            "world!".to_string(),
        ];
        sender.respond_stream(&task, chunks).await.unwrap();

        let received = receiver.poll_stream_chunks(&task.id).await.unwrap();
        assert_eq!(received.len(), 3);
        assert_eq!(received[0].text, "Hello ");
        assert_eq!(received[1].text, "streaming ");
        assert_eq!(received[2].text, "world!");
        assert!(received[2].is_final);

        // Reassemble
        let full = receiver.reassemble_stream(&task.id);
        assert_eq!(full, Some("Hello streaming world!".to_string()));
    }

    #[tokio::test]
    async fn reassemble_with_empty_chunk_texts() {
        let transport = InMemoryTransport::new();
        let task_id = "empty-chunks";
        let stream_topic = topics::stream_topic(task_id);

        for (idx, text, is_final) in [(0, "", false), (1, "", false), (2, "end", true)] {
            let chunk = TaskStreamChunk {
                task_id: task_id.to_string(),
                chunk_index: idx,
                text: text.to_string(),
                is_final,
            };
            let envelope = A2AEnvelope::StreamChunk(chunk);
            let payload = serde_json::to_vec(&envelope).unwrap();
            transport.publish(&stream_topic, &payload).await.unwrap();
        }

        let node = make_node_with_transport("receiver", transport);
        node.poll_stream_chunks(task_id).await.unwrap();
        let result = node.reassemble_stream(task_id);
        assert_eq!(result, Some("end".to_string()));
    }

    #[tokio::test]
    async fn multiple_tasks_stream_independently() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport.clone());

        let task_a = Task::new(node.pubkey(), "02a", "task a");
        let task_b = Task::new(node.pubkey(), "02b", "task b");

        node.respond_stream(&task_a, vec!["A1".into(), "A2".into()])
            .await
            .unwrap();
        node.respond_stream(&task_b, vec!["B1".into(), "B2".into(), "B3".into()])
            .await
            .unwrap();

        let receiver = make_node_with_transport("receiver", transport);

        let chunks_a = receiver.poll_stream_chunks(&task_a.id).await.unwrap();
        let chunks_b = receiver.poll_stream_chunks(&task_b.id).await.unwrap();

        assert_eq!(chunks_a.len(), 2);
        assert_eq!(chunks_a[0].text, "A1");
        assert_eq!(chunks_a[1].text, "A2");
        assert!(chunks_a[1].is_final);

        assert_eq!(chunks_b.len(), 3);
        assert_eq!(chunks_b[0].text, "B1");
        assert_eq!(chunks_b[2].text, "B3");
        assert!(chunks_b[2].is_final);

        // Reassemble both independently
        assert_eq!(
            receiver.reassemble_stream(&task_a.id),
            Some("A1A2".to_string())
        );
        assert_eq!(
            receiver.reassemble_stream(&task_b.id),
            Some("B1B2B3".to_string())
        );
    }

    #[tokio::test]
    async fn poll_stream_accumulates_across_calls() {
        let transport = InMemoryTransport::new();
        let task_id = "incremental";
        let stream_topic = topics::stream_topic(task_id);

        // First batch: chunk 0
        let chunk = TaskStreamChunk {
            task_id: task_id.to_string(),
            chunk_index: 0,
            text: "first".to_string(),
            is_final: false,
        };
        let envelope = A2AEnvelope::StreamChunk(chunk);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        let node = make_node_with_transport("receiver", transport.clone());
        let chunks = node.poll_stream_chunks(task_id).await.unwrap();
        assert_eq!(chunks.len(), 1);

        // Second batch: chunk 1 (final)
        let chunk = TaskStreamChunk {
            task_id: task_id.to_string(),
            chunk_index: 1,
            text: "second".to_string(),
            is_final: true,
        };
        let envelope = A2AEnvelope::StreamChunk(chunk);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        let chunks = node.poll_stream_chunks(task_id).await.unwrap();
        // Should have both chunks accumulated
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "first");
        assert_eq!(chunks[1].text, "second");
        assert!(chunks[1].is_final);

        // Reassemble should work now
        assert_eq!(
            node.reassemble_stream(task_id),
            Some("firstsecond".to_string())
        );
    }

    #[tokio::test]
    async fn respond_stream_large_chunk_count() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport.clone());
        let task = Task::new(node.pubkey(), "02recipient", "many chunks");

        let chunks: Vec<String> = (0..100).map(|i| format!("{i}")).collect();
        node.respond_stream(&task, chunks).await.unwrap();

        let receiver = make_node_with_transport("receiver", transport);
        let received = receiver.poll_stream_chunks(&task.id).await.unwrap();
        assert_eq!(received.len(), 100);
        assert_eq!(received[0].chunk_index, 0);
        assert_eq!(received[99].chunk_index, 99);
        assert!(received[99].is_final);
        assert!(!received[98].is_final);
    }
}
