//! Streaming operations for [`WakuA2ANode`](crate::WakuA2ANode).

use logos_messaging_a2a_core::{topics, A2AEnvelope, Task, TaskStreamChunk};
use logos_messaging_a2a_transport::Transport;

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
}
