//! Shared test infrastructure for task submodule tests.

use anyhow::Result;
use async_trait::async_trait;
use logos_messaging_a2a_core::AgentCard;
use logos_messaging_a2a_execution::{AgentId, ExecutionBackend, TransferDetails, TxHash};
use logos_messaging_a2a_storage::StorageBackend;
use logos_messaging_a2a_transport::Transport;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub type PublishedMessages = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

pub struct MockTransport {
    pub published: PublishedMessages,
    pub state: Arc<Mutex<MockState>>,
}

pub struct MockState {
    pub subscribers: HashMap<String, Vec<mpsc::Sender<Vec<u8>>>>,
    pub history: HashMap<String, Vec<Vec<u8>>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self {
            published: Arc::new(Mutex::new(Vec::new())),
            state: Arc::new(Mutex::new(MockState {
                subscribers: HashMap::new(),
                history: HashMap::new(),
            })),
        }
    }

    pub fn inject(&self, topic: &str, payload: Vec<u8>) {
        let mut state = self.state.lock().unwrap();
        state
            .history
            .entry(topic.to_string())
            .or_default()
            .push(payload.clone());
        if let Some(subs) = state.subscribers.get_mut(topic) {
            subs.retain(|tx| tx.try_send(payload.clone()).is_ok());
        }
    }
}

impl Clone for MockTransport {
    fn clone(&self) -> Self {
        Self {
            published: self.published.clone(),
            state: self.state.clone(),
        }
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let data = payload.to_vec();
        self.published
            .lock()
            .unwrap()
            .push((topic.to_string(), data.clone()));

        let mut state = self.state.lock().unwrap();
        state
            .history
            .entry(topic.to_string())
            .or_default()
            .push(data.clone());
        if let Some(subs) = state.subscribers.get_mut(topic) {
            subs.retain(|tx| tx.try_send(data.clone()).is_ok());
        }
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        let mut state = self.state.lock().unwrap();
        let (tx, rx) = mpsc::channel(1024);
        if let Some(history) = state.history.get(topic) {
            for msg in history {
                let _ = tx.try_send(msg.clone());
            }
        }
        state
            .subscribers
            .entry(topic.to_string())
            .or_default()
            .push(tx);
        Ok(rx)
    }

    async fn unsubscribe(&self, topic: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.subscribers.remove(topic);
        Ok(())
    }
}

pub struct MockStorage {
    pub store: Mutex<HashMap<String, Vec<u8>>>,
    pub next_id: Mutex<u64>,
}

impl MockStorage {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
            next_id: Mutex::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.store.lock().unwrap().len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.store.lock().unwrap().is_empty()
    }
}

#[async_trait]
impl StorageBackend for MockStorage {
    async fn upload(
        &self,
        data: Vec<u8>,
    ) -> Result<String, logos_messaging_a2a_storage::StorageError> {
        let mut id = self.next_id.lock().unwrap();
        let cid = format!("zMock{}", *id);
        *id += 1;
        self.store.lock().unwrap().insert(cid.clone(), data);
        Ok(cid)
    }

    async fn download(
        &self,
        cid: &str,
    ) -> Result<Vec<u8>, logos_messaging_a2a_storage::StorageError> {
        self.store.lock().unwrap().get(cid).cloned().ok_or_else(|| {
            logos_messaging_a2a_storage::StorageError::Api {
                status: 404,
                body: format!("CID not found: {}", cid),
            }
        })
    }
}

pub struct MockExecutionBackend;

#[async_trait]
impl ExecutionBackend for MockExecutionBackend {
    async fn register_agent(&self, _card: &AgentCard) -> anyhow::Result<TxHash> {
        Ok(TxHash([0; 32]))
    }
    async fn pay(&self, _to: &AgentId, _amount: u64) -> anyhow::Result<TxHash> {
        Ok(TxHash([0xab; 32]))
    }
    async fn balance(&self, _agent: &AgentId) -> anyhow::Result<u64> {
        Ok(1000)
    }
    async fn verify_transfer(&self, _tx_hash: &str) -> anyhow::Result<TransferDetails> {
        Ok(TransferDetails {
            from: "0xsender".into(),
            to: "0xrecipient".into(),
            amount: 100,
            block_number: 1,
        })
    }
}

/// Mock backend that returns configurable transfer details for verify_transfer.
pub struct VerifyingBackend {
    pub details: TransferDetails,
}

#[async_trait]
impl ExecutionBackend for VerifyingBackend {
    async fn register_agent(&self, _card: &AgentCard) -> anyhow::Result<TxHash> {
        Ok(TxHash([0; 32]))
    }
    async fn pay(&self, _to: &AgentId, _amount: u64) -> anyhow::Result<TxHash> {
        Ok(TxHash([0; 32]))
    }
    async fn balance(&self, _agent: &AgentId) -> anyhow::Result<u64> {
        Ok(0)
    }
    async fn verify_transfer(&self, _tx_hash: &str) -> anyhow::Result<TransferDetails> {
        Ok(self.details.clone())
    }
}

/// Mock backend where `pay()` always fails.
pub struct FailingPayBackend;

#[async_trait]
impl ExecutionBackend for FailingPayBackend {
    async fn register_agent(&self, _card: &AgentCard) -> anyhow::Result<TxHash> {
        Ok(TxHash([0; 32]))
    }
    async fn pay(&self, _to: &AgentId, _amount: u64) -> anyhow::Result<TxHash> {
        anyhow::bail!("payment failed: insufficient funds")
    }
    async fn balance(&self, _agent: &AgentId) -> anyhow::Result<u64> {
        Ok(0)
    }
    async fn verify_transfer(&self, _tx_hash: &str) -> anyhow::Result<TransferDetails> {
        Ok(TransferDetails {
            from: String::new(),
            to: String::new(),
            amount: 0,
            block_number: 0,
        })
    }
}

/// Mock backend where verify_transfer always fails.
pub struct FailingVerifyBackend;

#[async_trait]
impl ExecutionBackend for FailingVerifyBackend {
    async fn register_agent(&self, _card: &AgentCard) -> anyhow::Result<TxHash> {
        Ok(TxHash([0; 32]))
    }
    async fn pay(&self, _to: &AgentId, _amount: u64) -> anyhow::Result<TxHash> {
        Ok(TxHash([0; 32]))
    }
    async fn balance(&self, _agent: &AgentId) -> anyhow::Result<u64> {
        Ok(0)
    }
    async fn verify_transfer(&self, tx_hash: &str) -> anyhow::Result<TransferDetails> {
        anyhow::bail!("Transaction {} not found", tx_hash)
    }
}

/// Storage backend that always fails on upload.
pub struct FailingUploadStorage;

#[async_trait]
impl StorageBackend for FailingUploadStorage {
    async fn upload(
        &self,
        _data: Vec<u8>,
    ) -> Result<String, logos_messaging_a2a_storage::StorageError> {
        Err(logos_messaging_a2a_storage::StorageError::Http(
            "upload failed".to_string(),
        ))
    }

    async fn download(
        &self,
        _cid: &str,
    ) -> Result<Vec<u8>, logos_messaging_a2a_storage::StorageError> {
        Err(logos_messaging_a2a_storage::StorageError::Http(
            "download failed".to_string(),
        ))
    }
}

/// Storage backend that always fails on download.
pub struct FailingDownloadStorage {
    store: Mutex<HashMap<String, Vec<u8>>>,
    next_id: Mutex<u64>,
}

impl FailingDownloadStorage {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
            next_id: Mutex::new(0),
        }
    }
}

#[async_trait]
impl StorageBackend for FailingDownloadStorage {
    async fn upload(
        &self,
        data: Vec<u8>,
    ) -> Result<String, logos_messaging_a2a_storage::StorageError> {
        let mut id = self.next_id.lock().unwrap();
        let cid = format!("zFail{}", *id);
        *id += 1;
        self.store.lock().unwrap().insert(cid.clone(), data);
        Ok(cid)
    }

    async fn download(
        &self,
        _cid: &str,
    ) -> Result<Vec<u8>, logos_messaging_a2a_storage::StorageError> {
        Err(logos_messaging_a2a_storage::StorageError::Api {
            status: 500,
            body: "download always fails".to_string(),
        })
    }
}

pub fn fast_config() -> logos_messaging_a2a_transport::sds::ChannelConfig {
    logos_messaging_a2a_transport::sds::ChannelConfig {
        ack_timeout: std::time::Duration::from_millis(1),
        max_retries: 0,
        ..Default::default()
    }
}
