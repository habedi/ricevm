#![allow(dead_code)]
//! Channel implementation for inter-thread communication.
//!
//! Supports both single-slot buffered payloads (used by VmState directly)
//! and queue-based blocking channels (used by the preemptive scheduler).

use std::collections::VecDeque;

/// A pending send or receive request on a channel.
#[derive(Debug)]
pub(crate) struct ChannelRequest {
    /// The thread ID that made the request.
    pub thread_id: u32,
    /// The data being sent (for send requests) or buffer for received data.
    pub data: Vec<u8>,
}

/// A Dis VM channel for inter-thread communication.
#[derive(Debug)]
pub(crate) struct Channel {
    /// Pending send requests (data waiting to be received).
    pub senders: VecDeque<ChannelRequest>,
    /// Pending receive requests (threads waiting for data).
    pub receivers: VecDeque<ChannelRequest>,
    /// Element size in bytes.
    pub elem_size: usize,
}

impl Channel {
    pub fn new(elem_size: usize) -> Self {
        Self {
            senders: VecDeque::new(),
            receivers: VecDeque::new(),
            elem_size,
        }
    }

    /// Try to send data. Returns Some(receiver_request) if a receiver was matched.
    pub fn try_send(&mut self, data: Vec<u8>, thread_id: u32) -> Option<ChannelRequest> {
        if let Some(receiver) = self.receivers.pop_front() {
            // Match with waiting receiver
            Some(ChannelRequest {
                thread_id: receiver.thread_id,
                data,
            })
        } else {
            // Queue the send
            self.senders.push_back(ChannelRequest { thread_id, data });
            None
        }
    }

    /// Try to receive. Returns Some(data) if a sender was matched, None if queued.
    pub fn try_recv(&mut self, thread_id: u32) -> Option<Vec<u8>> {
        if let Some(sender) = self.senders.pop_front() {
            Some(sender.data)
        } else {
            // Queue the receive
            self.receivers.push_back(ChannelRequest {
                thread_id,
                data: Vec::new(),
            });
            None
        }
    }

    /// Check if there is data available to receive without blocking.
    pub fn has_pending_send(&self) -> bool {
        !self.senders.is_empty()
    }

    /// Check if there is a receiver waiting.
    pub fn has_pending_recv(&self) -> bool {
        !self.receivers.is_empty()
    }
}

/// Channel table for the preemptive scheduler.
/// Maps channel HeapIds to their queue state.
#[derive(Debug, Default)]
pub(crate) struct ChannelTable {
    channels: std::collections::HashMap<u32, Channel>,
}

impl ChannelTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create a channel for a given HeapId.
    pub fn get_or_create(&mut self, id: u32, elem_size: usize) -> &mut Channel {
        self.channels
            .entry(id)
            .or_insert_with(|| Channel::new(elem_size))
    }

    /// Remove a channel.
    pub fn remove(&mut self, id: u32) {
        self.channels.remove(&id);
    }
}
