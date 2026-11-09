use super::SessionEvent;
use crate::client::Association;
use anyhow::{Result, anyhow, ensure};
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch};

pub fn channel() -> (SessionEventSender, SessionEventReceiver) {
    let (sender, receiver) = mpsc::channel(32);
    let (display_closed, _) = watch::channel(false);
    (
        SessionEventSender {
            sender,
            bytes: Arc::new(Semaphore::new(8 * 1024 * 1024)),
            display_closed: display_closed.clone(),
        },
        SessionEventReceiver {
            receiver,
            display_closed,
        },
    )
}
pub struct SessionEventPacket {
    pub event: SessionEvent,
    pub association: Option<Association>,
    pub request_id: Option<u64>,
    _bytes: OwnedSemaphorePermit,
}
#[derive(Clone)]
pub struct SessionEventSender {
    sender: mpsc::Sender<SessionEventPacket>,
    bytes: Arc<Semaphore>,
    display_closed: watch::Sender<bool>,
}
impl SessionEventSender {
    pub async fn send(&self, event: SessionEvent) -> Result<()> {
        self.enqueue(event, None, None).await
    }
    pub async fn send_associated(
        &self,
        association: Association,
        event: SessionEvent,
    ) -> Result<()> {
        self.enqueue(event, None, Some(association)).await
    }
    pub async fn send_reply(
        &self,
        id: u64,
        association: Option<Association>,
        event: SessionEvent,
    ) -> Result<()> {
        self.enqueue(event, Some(id), association).await
    }
    pub fn try_send_reply(&self, id: u64, event: SessionEvent) -> Result<()> {
        let result = (|| {
            ensure!(!self.is_closed(), "display closed");
            let size = event_size(&event)?;
            let permit = self.bytes.clone().try_acquire_many_owned(size)?;
            self.sender
                .try_send(SessionEventPacket {
                    event,
                    association: None,
                    request_id: Some(id),
                    _bytes: permit,
                })
                .map_err(|e| anyhow!("{e}"))?;
            Ok(())
        })();
        if result.is_err() {
            self.display_closed.send_replace(true);
        }
        result
    }
    pub fn is_closed(&self) -> bool {
        *self.display_closed.borrow() || self.sender.is_closed()
    }
    pub async fn closed(&self) {
        tokio::select! {
            _ = wait_closed(&self.display_closed) => {},
            _ = self.sender.closed() => {},
        }
    }
    async fn enqueue(
        &self,
        event: SessionEvent,
        request_id: Option<u64>,
        association: Option<Association>,
    ) -> Result<()> {
        let delivery = async {
            ensure!(!self.is_closed(), "display closed");
            let size = event_size(&event)?;
            let permit = self.bytes.clone().acquire_many_owned(size).await?;
            self.sender
                .send(SessionEventPacket {
                    event,
                    association,
                    request_id,
                    _bytes: permit,
                })
                .await
                .map_err(|_| anyhow!("display closed"))?;
            Ok::<(), anyhow::Error>(())
        };
        let result = tokio::select! {
            biased;
            _ = self.closed() => Err(anyhow!("display closed")),
            result = tokio::time::timeout(Duration::from_secs(30), delivery) => {
                result.unwrap_or_else(|_| Err(anyhow!("display enqueue timed out")))
            }
        };
        if result.is_err() {
            // Missing a terminal or control reply invalidates the whole display stream.
            self.display_closed.send_replace(true);
        }
        result
    }
}
fn event_size(event: &SessionEvent) -> Result<u32> {
    let size = serde_json::to_vec(event)?.len();
    ensure!(size <= 8 * 1024 * 1024, "terminal projection exceeds 8 MiB");
    Ok(size as u32)
}
pub struct SessionEventReceiver {
    receiver: mpsc::Receiver<SessionEventPacket>,
    display_closed: watch::Sender<bool>,
}
impl SessionEventReceiver {
    pub async fn recv(&mut self) -> Option<SessionEventPacket> {
        tokio::select! {
            biased;
            _ = wait_closed(&self.display_closed) => {
                self.receiver.close();
                while self.receiver.try_recv().is_ok() {}
                None
            },
            packet = self.receiver.recv() => packet,
        }
    }
}
impl Drop for SessionEventReceiver {
    fn drop(&mut self) {
        self.display_closed.send_replace(true);
    }
}
async fn wait_closed(state: &watch::Sender<bool>) {
    let mut state = state.subscribe();
    let _ = state.wait_for(|closed| *closed).await;
}

#[cfg(test)]
#[path = "../../tests/session/channel.rs"]
mod tests;
