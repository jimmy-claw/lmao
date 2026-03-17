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

    // --- metrics tests ---

    #[tokio::test]
    async fn respond_stream_increments_metrics() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport.clone());
        let task = Task::new(node.pubkey(), "02recipient", "do something");

        let chunks = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        node.respond_stream(&task, chunks).await.unwrap();

        let m = node.metrics();
        assert_eq!(m.stream_chunks_sent, 3);
        assert_eq!(m.messages_published, 3);
    }

    #[tokio::test]
    async fn poll_stream_chunks_increments_metrics() {
        let transport = InMemoryTransport::new();
        let task_id = "metrics-task";
        let stream_topic = topics::stream_topic(task_id);

        for i in 0..2 {
            let chunk = TaskStreamChunk {
                task_id: task_id.to_string(),
                chunk_index: i,
                text: format!("chunk-{i}"),
                is_final: i == 1,
            };
            let envelope = A2AEnvelope::StreamChunk(chunk);
            let payload = serde_json::to_vec(&envelope).unwrap();
            transport.publish(&stream_topic, &payload).await.unwrap();
        }

        let node = make_node_with_transport("receiver", transport);
        node.poll_stream_chunks(task_id).await.unwrap();

        assert_eq!(node.metrics().stream_chunks_received, 2);
    }

    // --- multiple polls accumulate ---

    #[tokio::test]
    async fn multiple_polls_accumulate_chunks() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("receiver", transport.clone());
        let task_id = "accumulate-task";
        let stream_topic = topics::stream_topic(task_id);

        // First batch
        let chunk0 = TaskStreamChunk {
            task_id: task_id.to_string(),
            chunk_index: 0,
            text: "Hello ".to_string(),
            is_final: false,
        };
        let payload = serde_json::to_vec(&A2AEnvelope::StreamChunk(chunk0)).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        let result1 = node.poll_stream_chunks(task_id).await.unwrap();
        assert_eq!(result1.len(), 1);

        // Second batch
        let chunk1 = TaskStreamChunk {
            task_id: task_id.to_string(),
            chunk_index: 1,
            text: "world!".to_string(),
            is_final: true,
        };
        let payload = serde_json::to_vec(&A2AEnvelope::StreamChunk(chunk1)).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        let result2 = node.poll_stream_chunks(task_id).await.unwrap();
        assert_eq!(result2.len(), 2);
        assert_eq!(result2[0].text, "Hello ");
        assert_eq!(result2[1].text, "world!");
        assert!(result2[1].is_final);
    }

    // --- end-to-end: respond_stream -> poll -> reassemble ---

    #[tokio::test]
    async fn end_to_end_stream_roundtrip() {
        let transport = InMemoryTransport::new();
        let sender = make_node_with_transport("sender", transport.clone());
        let receiver = make_node_with_transport("receiver", transport.clone());

        let task = Task::new(sender.pubkey(), receiver.pubkey(), "stream me");

        sender
            .respond_stream(
                &task,
                vec![
                    "The ".to_string(),
                    "quick ".to_string(),
                    "brown ".to_string(),
                    "fox".to_string(),
                ],
            )
            .await
            .unwrap();

        let chunks = receiver.poll_stream_chunks(&task.id).await.unwrap();
        assert_eq!(chunks.len(), 4);
        assert!(chunks[3].is_final);

        let reassembled = receiver.reassemble_stream(&task.id);
        assert_eq!(reassembled, Some("The quick brown fox".to_string()));
    }

    // --- two tasks don't interfere ---

    #[tokio::test]
    async fn separate_tasks_have_independent_streams() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport.clone());
        let receiver = make_node_with_transport("receiver", transport.clone());

        let task_a = Task::new(node.pubkey(), receiver.pubkey(), "task A");
        let task_b = Task::new(node.pubkey(), receiver.pubkey(), "task B");

        node.respond_stream(&task_a, vec!["AAA".to_string()])
            .await
            .unwrap();
        node.respond_stream(&task_b, vec!["BBB".to_string()])
            .await
            .unwrap();

        receiver.poll_stream_chunks(&task_a.id).await.unwrap();
        receiver.poll_stream_chunks(&task_b.id).await.unwrap();

        assert_eq!(
            receiver.reassemble_stream(&task_a.id),
            Some("AAA".to_string())
        );
        assert_eq!(
            receiver.reassemble_stream(&task_b.id),
            Some("BBB".to_string())
        );
    }

    // --- non-StreamChunk envelopes ignored on stream topic ---

    #[tokio::test]
    async fn poll_stream_ignores_non_stream_chunk_envelopes() {
        let transport = InMemoryTransport::new();
        let task_id = "filter-task";
        let stream_topic = topics::stream_topic(task_id);

        // Inject a Task envelope on the stream topic
        let task = Task::new("from", "to", "hello");
        let envelope = A2AEnvelope::Task(task);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        // Also inject garbage
        transport
            .publish(&stream_topic, b"not json at all")
            .await
            .unwrap();

        let node = make_node_with_transport("receiver", transport);
        let chunks = node.poll_stream_chunks(task_id).await.unwrap();
        assert!(chunks.is_empty());
    }

    // --- empty chunks vec ---

    #[tokio::test]
    async fn respond_stream_empty_chunks_is_noop() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport.clone());
        let task = Task::new(node.pubkey(), "02recipient", "do something");

        // Empty vec should not panic
        node.respond_stream(&task, vec![]).await.unwrap();

        let m = node.metrics();
        assert_eq!(m.stream_chunks_sent, 0);
        assert_eq!(m.messages_published, 0);
    }

    // --- reassemble edge cases ---

    #[tokio::test]
    async fn reassemble_returns_none_for_empty_buffer() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("receiver", transport.clone());

        // Poll with no messages — creates empty buffer entry
        let task_id = "empty-buffer-task";
        node.poll_stream_chunks(task_id).await.unwrap();
        assert!(node.reassemble_stream(task_id).is_none());
    }

    #[tokio::test]
    async fn reassemble_single_final_chunk() {
        let transport = InMemoryTransport::new();
        let task_id = "single-final";
        let stream_topic = topics::stream_topic(task_id);

        let chunk = TaskStreamChunk {
            task_id: task_id.to_string(),
            chunk_index: 0,
            text: "complete".to_string(),
            is_final: true,
        };
        let payload = serde_json::to_vec(&A2AEnvelope::StreamChunk(chunk)).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        let node = make_node_with_transport("receiver", transport);
        node.poll_stream_chunks(task_id).await.unwrap();
        assert_eq!(
            node.reassemble_stream(task_id),
            Some("complete".to_string())
        );
    }

    // --- large number of chunks ---

    #[tokio::test]
    async fn respond_stream_many_chunks() {
        let transport = InMemoryTransport::new();
        let sender = make_node_with_transport("sender", transport.clone());
        let receiver = make_node_with_transport("receiver", transport.clone());
        let task = Task::new(sender.pubkey(), receiver.pubkey(), "big stream");

        let chunks: Vec<String> = (0..100).map(|i| format!("{i} ")).collect();
        sender.respond_stream(&task, chunks).await.unwrap();

        let received = receiver.poll_stream_chunks(&task.id).await.unwrap();
        assert_eq!(received.len(), 100);
        assert_eq!(received[0].chunk_index, 0);
        assert_eq!(received[99].chunk_index, 99);
        assert!(received[99].is_final);
        assert!(!received[98].is_final);

        let reassembled = receiver.reassemble_stream(&task.id).unwrap();
        let expected: String = (0..100).map(|i| format!("{i} ")).collect();
        assert_eq!(reassembled, expected);

        assert_eq!(sender.metrics().stream_chunks_sent, 100);
    }

    // --- respond_stream chunk_index and is_final correctness ---

    #[tokio::test]
    async fn respond_stream_sets_correct_indices_and_final_flag() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport.clone());
        let task = Task::new(node.pubkey(), "02recipient", "test");

        node.respond_stream(&task, vec!["a".into(), "b".into(), "c".into()])
            .await
            .unwrap();

        let stream_topic = topics::stream_topic(&task.id);
        let mut rx = transport.subscribe(&stream_topic).await.unwrap();

        let expected = [(0u32, "a", false), (1, "b", false), (2, "c", true)];
        for (idx, text, is_final) in expected {
            let msg = rx.try_recv().unwrap();
            if let A2AEnvelope::StreamChunk(chunk) = serde_json::from_slice(&msg).unwrap() {
                assert_eq!(chunk.chunk_index, idx);
                assert_eq!(chunk.text, text);
                assert_eq!(chunk.is_final, is_final);
                assert_eq!(chunk.task_id, task.id);
            } else {
                panic!("expected StreamChunk");
            }
        }
        assert!(rx.try_recv().is_err());
    }

    // --- poll dedup across multiple polls ---

    #[tokio::test]
    async fn poll_deduplicates_across_multiple_polls() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("receiver", transport.clone());
        let task_id = "dedup-multi";
        let stream_topic = topics::stream_topic(task_id);

        // Inject chunk 0
        let chunk0 = TaskStreamChunk {
            task_id: task_id.to_string(),
            chunk_index: 0,
            text: "first".to_string(),
            is_final: false,
        };
        let payload = serde_json::to_vec(&A2AEnvelope::StreamChunk(chunk0.clone())).unwrap();
        transport.publish(&stream_topic, &payload).await.unwrap();

        // First poll picks it up
        let r1 = node.poll_stream_chunks(task_id).await.unwrap();
        assert_eq!(r1.len(), 1);

        // Inject same chunk 0 again + new chunk 1
        transport.publish(&stream_topic, &payload).await.unwrap();
        let chunk1 = TaskStreamChunk {
            task_id: task_id.to_string(),
            chunk_index: 1,
            text: "second".to_string(),
            is_final: true,
        };
        let payload1 = serde_json::to_vec(&A2AEnvelope::StreamChunk(chunk1)).unwrap();
        transport.publish(&stream_topic, &payload1).await.unwrap();

        // Second poll should dedup chunk 0 and add chunk 1
        let r2 = node.poll_stream_chunks(task_id).await.unwrap();
        assert_eq!(r2.len(), 2);
        assert_eq!(r2[0].text, "first");
        assert_eq!(r2[1].text, "second");
    }

    // --- stream topic format ---

    #[tokio::test]
    async fn respond_stream_uses_correct_topic() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("streamer", transport.clone());
        let task = Task::new(node.pubkey(), "02recipient", "test");

        node.respond_stream(&task, vec!["hello".into()])
            .await
            .unwrap();

        // Verify published on the correct stream topic
        let expected_topic = format!("/waku-a2a/1/stream/{}/proto", task.id);
        let mut rx = transport.subscribe(&expected_topic).await.unwrap();
        assert!(rx.try_recv().is_ok());
    }
}
