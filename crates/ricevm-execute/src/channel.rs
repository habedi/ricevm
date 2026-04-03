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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_new_is_empty() {
        let ch = Channel::new(4);
        assert_eq!(ch.elem_size, 4);
        assert!(!ch.has_pending_send());
        assert!(!ch.has_pending_recv());
    }

    #[test]
    fn try_send_queues_when_no_receiver() {
        let mut ch = Channel::new(4);
        let result = ch.try_send(vec![1, 2, 3, 4], 1);
        assert!(result.is_none(), "no receiver, should queue");
        assert!(ch.has_pending_send());
        assert!(!ch.has_pending_recv());
    }

    #[test]
    fn try_recv_queues_when_no_sender() {
        let mut ch = Channel::new(4);
        let result = ch.try_recv(1);
        assert!(result.is_none(), "no sender, should queue");
        assert!(ch.has_pending_recv());
        assert!(!ch.has_pending_send());
    }

    #[test]
    fn send_then_recv_matches() {
        let mut ch = Channel::new(4);
        let data = vec![10, 20, 30, 40];
        ch.try_send(data.clone(), 1);
        let received = ch.try_recv(2);
        assert_eq!(received, Some(data));
        assert!(!ch.has_pending_send());
        assert!(!ch.has_pending_recv());
    }

    #[test]
    fn recv_then_send_matches() {
        let mut ch = Channel::new(4);
        ch.try_recv(2); // receiver waits
        assert!(ch.has_pending_recv());

        let data = vec![5, 6, 7, 8];
        let matched = ch.try_send(data.clone(), 1);
        assert!(matched.is_some());
        let req = matched.unwrap();
        assert_eq!(req.thread_id, 2); // receiver's thread id
        assert_eq!(req.data, data);
        assert!(!ch.has_pending_recv());
    }

    #[test]
    fn fifo_ordering_for_multiple_sends() {
        let mut ch = Channel::new(4);
        ch.try_send(vec![1, 0, 0, 0], 1);
        ch.try_send(vec![2, 0, 0, 0], 2);
        ch.try_send(vec![3, 0, 0, 0], 3);

        assert_eq!(ch.try_recv(10), Some(vec![1, 0, 0, 0]));
        assert_eq!(ch.try_recv(11), Some(vec![2, 0, 0, 0]));
        assert_eq!(ch.try_recv(12), Some(vec![3, 0, 0, 0]));
        assert!(!ch.has_pending_send());
    }

    #[test]
    fn fifo_ordering_for_multiple_recvs() {
        let mut ch = Channel::new(4);
        ch.try_recv(10);
        ch.try_recv(11);

        let matched1 = ch.try_send(vec![1, 0, 0, 0], 1);
        assert_eq!(matched1.as_ref().unwrap().thread_id, 10);

        let matched2 = ch.try_send(vec![2, 0, 0, 0], 2);
        assert_eq!(matched2.as_ref().unwrap().thread_id, 11);

        assert!(!ch.has_pending_recv());
    }

    // --- ChannelTable ---

    #[test]
    fn channel_table_get_or_create_creates_new() {
        let mut table = ChannelTable::new();
        let ch = table.get_or_create(100, 8);
        assert_eq!(ch.elem_size, 8);
        assert!(!ch.has_pending_send());
    }

    #[test]
    fn channel_table_get_or_create_returns_existing() {
        let mut table = ChannelTable::new();
        table.get_or_create(100, 8).try_send(vec![1, 2], 1);
        // Get again - should have the pending send
        assert!(table.get_or_create(100, 8).has_pending_send());
    }

    #[test]
    fn channel_table_remove_deletes_channel() {
        let mut table = ChannelTable::new();
        table.get_or_create(100, 4);
        table.remove(100);
        // After remove, get_or_create should give a fresh channel
        let ch = table.get_or_create(100, 4);
        assert!(!ch.has_pending_send());
        assert!(!ch.has_pending_recv());
    }
}
