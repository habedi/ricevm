#![allow(dead_code)]
//! Channel implementation for inter-thread communication.
//!
//! Channels in Dis are typed, synchronous (unbuffered) communication endpoints.
//! A send blocks until a receiver is ready, and vice versa.

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

    /// Try to send data. Returns true if a receiver was matched, false if queued.
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
}
