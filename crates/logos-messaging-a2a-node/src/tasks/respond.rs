use logos_messaging_a2a_core::{topics, AgentCard, Task};
use logos_messaging_a2a_transport::Transport;

use crate::metrics::Metrics;
use crate::{Result, WakuA2ANode};

impl<T: Transport> WakuA2ANode<T> {
    /// Respond to a task: send back a completed task with result.
    ///
    /// Uses SDS causal send (maintains ordering, includes bloom filter
    /// for implicit ACK) but does not block on explicit ACK.
    pub async fn respond(&self, task: &Task, result_text: &str) -> Result<()> {
        self.respond_to(task, result_text, None).await
    }

    /// Respond to a task, optionally encrypting to the sender.
    pub async fn respond_to(
        &self,
        task: &Task,
        result_text: &str,
        sender_card: Option<&AgentCard>,
    ) -> Result<()> {
        let response = task.respond(result_text);
        let topic = topics::task_topic(&response.to);
        let payload = self.prepare_payload(&response, sender_card).await?;

        // Use causal send for responses (maintains ordering, no retransmit block)
        self.channel.send(&topic, &payload).await?;
        Metrics::inc(&self.metrics.responses_sent);
        Metrics::inc(&self.metrics.messages_published);

        tracing::info!(task_id = %task.id, "Responded to task");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::tasks::test_support::{fast_config, MockTransport};
    use crate::WakuA2ANode;
    use logos_messaging_a2a_core::{topics, Task};

    #[tokio::test]
    async fn test_respond_publishes_to_sender_topic() {
        let transport = MockTransport::new();
        let published = transport.published.clone();

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        );
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let spk = sender.pubkey().to_string();
        let _ = sender.poll_tasks().await.unwrap();

        // Sender sends task to receiver
        let task = Task::new(&spk, &rpk, "question?");
        sender.send_task(&task).await.unwrap();

        let tasks = receiver.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);

        // Record message count before respond
        let pre_count = published.lock().unwrap().len();
        receiver.respond(&tasks[0], "answer!").await.unwrap();
        let post_count = published.lock().unwrap().len();
        assert!(post_count > pre_count, "respond should publish a message");

        // Verify the response was published to the SENDER's task topic
        let sender_topic = topics::task_topic(&spk);
        let pubs = published.lock().unwrap();
        let to_sender = pubs.iter().filter(|(t, _)| *t == sender_topic).count();
        assert!(to_sender >= 1, "response should target sender's topic");
    }

    #[tokio::test]
    async fn test_respond_to_encrypted_publishes() {
        let transport = MockTransport::new();
        let published = transport.published.clone();

        let receiver = WakuA2ANode::new_encrypted(
            "enc-receiver",
            "encrypted receiver",
            vec![],
            transport.clone(),
        );
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::new_encrypted("enc-sender", "encrypted sender", vec![], transport.clone());
        let spk = sender.pubkey().to_string();
        let _ = sender.poll_tasks().await.unwrap();

        // Send encrypted task
        let task = Task::new(&spk, &rpk, "secret question");
        sender
            .send_task_to(&task, Some(&receiver.card))
            .await
            .unwrap();

        let tasks = receiver.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text(), Some("secret question"));

        // Respond with encryption — verify it publishes to sender's topic
        let pre_count = published.lock().unwrap().len();
        receiver
            .respond_to(&tasks[0], "secret answer", Some(&sender.card))
            .await
            .unwrap();
        let post_count = published.lock().unwrap().len();
        assert!(post_count > pre_count, "encrypted respond should publish");

        let sender_topic = topics::task_topic(&spk);
        let pubs = published.lock().unwrap();
        let to_sender = pubs.iter().filter(|(t, _)| *t == sender_topic).count();
        assert!(
            to_sender >= 1,
            "encrypted response should target sender's topic"
        );
    }

    #[tokio::test]
    async fn test_multiple_responses_ordering() {
        let transport = MockTransport::new();

        let server =
            WakuA2ANode::with_config("server", "server", vec![], transport.clone(), fast_config());
        let spk = server.pubkey().to_string();
        let _ = server.poll_tasks().await.unwrap();

        let client =
            WakuA2ANode::with_config("client", "client", vec![], transport.clone(), fast_config());
        let cpk = client.pubkey().to_string();
        let _ = client.poll_tasks().await.unwrap();

        for i in 0..3 {
            let task = Task::new(&cpk, &spk, &format!("task-{i}"));
            client.send_task(&task).await.unwrap();
        }

        let tasks = server.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 3);

        for (i, task) in tasks.iter().enumerate() {
            server.respond(task, &format!("done-{i}")).await.unwrap();
        }

        let responses = client.poll_tasks().await.unwrap();
        assert_eq!(responses.len(), 3);
    }
}
