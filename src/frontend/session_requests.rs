//! Host-side request IDs and pending command ownership shared by terminal UIs.
//!
//! Commands are sent with `try_send` so a full bounded mailbox never blocks
//! display consumption; rejection leaves the caller's draft unchanged.

use std::collections::HashMap;

use tokio::sync::mpsc::{Sender, error::TrySendError};

use crate::{
    input::UserInput,
    session::{SessionCommand, SessionRequest},
};

#[derive(Clone, Debug)]
pub(crate) enum PendingRequest {
    /// A submitted message waiting for the Runtime acceptance reply. The draft
    /// is still the composer's; it is cleared only on acceptance.
    Send {
        session_id: String,
        message: UserInput,
    },
    Command {
        draft: String,
        session_id: Option<String>,
    },
    Control,
    Cancel {
        session_id: String,
        task_id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequestSendError {
    Full,
    Closed,
}

pub(crate) struct SessionRequests {
    sender: Option<Sender<SessionRequest>>,
    next: u64,
    pending: HashMap<u64, PendingRequest>,
}

impl SessionRequests {
    pub(crate) fn new(sender: Sender<SessionRequest>) -> Self {
        Self {
            sender: Some(sender),
            next: 1,
            pending: HashMap::new(),
        }
    }

    fn allocate(&mut self) -> u64 {
        let id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        id
    }

    /// Sends a request without blocking. Returns the host request id only when
    /// the bounded mailbox accepted it.
    pub(crate) fn try_send(
        &mut self,
        command: SessionCommand,
        pending: PendingRequest,
    ) -> Result<u64, RequestSendError> {
        let request_id = self.allocate();
        let Some(sender) = &self.sender else {
            return Err(RequestSendError::Closed);
        };
        match sender.try_send(SessionRequest {
            request_id,
            command,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(RequestSendError::Full),
            Err(TrySendError::Closed(_)) => {
                self.sender = None;
                return Err(RequestSendError::Closed);
            }
        }
        self.pending.insert(request_id, pending);
        Ok(request_id)
    }

    pub(crate) fn close(&mut self) {
        self.sender = None;
    }

    pub(crate) fn take(&mut self, request_id: u64) -> Option<PendingRequest> {
        self.pending.remove(&request_id)
    }

    pub(crate) fn has_pending_send(&self, session_id: &str) -> bool {
        self.pending.values().any(|pending| {
            matches!(pending, PendingRequest::Send { session_id: id, .. } if id == session_id)
        })
    }
}
