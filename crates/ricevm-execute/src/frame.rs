//! Frame stack management for the Dis VM.
//!
//! Frames are flat byte buffers stored contiguously. Each frame has a header
//! (prev_pc, prev_base) followed by the data area where locals and arguments
//! live. The `frame` instruction allocates a pending frame; `call` activates it.

use ricevm_core::{ExecError, Pc};

use crate::memory;

/// Size of the frame header in bytes: prev_pc (4) + prev_base (4) + reserved (8) = 16.
pub(crate) const FRAME_HEADER_SIZE: usize = 16;

/// Upper bound on the total size of the frame stack.
///
/// Frame offsets double as virtual addresses (see `address::decode_virtual_addr`),
/// so the stack must stay strictly below the module MP address range or a frame
/// pointer would decode as an MP reference. Exceeding it is a stack overflow.
pub(crate) const MAX_FRAME_STACK_SIZE: usize = crate::address::MP_BASE;

// Frame offsets must stay below the module MP address range.
const _: () = assert!(MAX_FRAME_STACK_SIZE <= crate::address::MP_BASE);

#[derive(Debug)]
pub(crate) struct FrameStack {
    /// Flat byte buffer holding all frames contiguously.
    pub data: Vec<u8>,
    /// Byte offset where the current (top) frame starts.
    pub(crate) current_base: usize,
    /// Total size of the current frame (header + data area).
    pub(crate) current_size: usize,
}

impl FrameStack {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            current_base: 0,
            current_size: 0,
        }
    }

    /// Push the initial entry frame directly (no two-phase needed for the first frame).
    pub fn push_entry(&mut self, data_area_size: usize, sentinel_pc: Pc) {
        let total = FRAME_HEADER_SIZE + data_area_size;
        self.data.resize(total, 0);
        // Write header
        memory::write_word(&mut self.data, 0, sentinel_pc); // prev_pc = -1 sentinel
        memory::write_word(&mut self.data, 4, 0); // prev_base = 0
        self.current_base = 0;
        self.current_size = total;
    }

    /// Byte offset where the current frame's data area starts (after header).
    pub fn current_data_offset(&self) -> usize {
        self.current_base + FRAME_HEADER_SIZE
    }

    /// Allocate space for a pending frame at the top of the stack.
    /// Returns the byte offset of the new frame's data area (absolute into `self.data`).
    /// The frame is NOT yet active; use `activate_pending` to make it current.
    pub fn alloc_pending(&mut self, data_area_size: usize) -> Result<usize, ExecError> {
        // Reserve space above everything already allocated. `data.len()` also
        // covers pending frames that have not been activated yet, so two
        // consecutive `frame` instructions get distinct, non-overlapping frames.
        let new_base = self
            .data
            .len()
            .max(self.current_base.saturating_add(self.current_size));
        let new_end = FRAME_HEADER_SIZE
            .checked_add(data_area_size)
            .and_then(|total| new_base.checked_add(total))
            .filter(|&end| end <= MAX_FRAME_STACK_SIZE)
            .ok_or_else(|| {
                ExecError::Other(format!("stack overflow: frame of {data_area_size} bytes"))
            })?;
        self.data.resize(new_end, 0);
        Ok(new_base + FRAME_HEADER_SIZE)
    }

    /// Make the pending frame (whose data area starts at `data_area_offset`)
    /// the current frame. Writes the frame header with the caller's saved PC.
    pub fn activate_pending(
        &mut self,
        data_area_offset: usize,
        saved_pc: Pc,
    ) -> Result<(), ExecError> {
        // `data_area_offset` comes from a bytecode operand (`call`/`spawn`), so
        // it must be validated before it is used to index the stack.
        if data_area_offset < FRAME_HEADER_SIZE || data_area_offset > self.data.len() {
            return Err(ExecError::Other(format!(
                "invalid frame pointer: {data_area_offset}"
            )));
        }
        let new_base = data_area_offset - FRAME_HEADER_SIZE;
        let new_size = self.data.len() - new_base;
        // Write header
        memory::write_word(&mut self.data, new_base, saved_pc);
        memory::write_word(&mut self.data, new_base + 4, self.current_base as i32);
        self.current_base = new_base;
        self.current_size = new_size;
        Ok(())
    }

    /// Pop the current frame, restoring the caller's frame.
    /// Returns the saved PC from the frame header.
    pub fn pop(&mut self) -> Result<Pc, ExecError> {
        if self.data.is_empty() {
            return Err(ExecError::Other("stack underflow".to_string()));
        }
        let prev_pc = memory::read_word(&self.data, self.current_base);
        let prev_base = memory::read_word(&self.data, self.current_base + 4) as usize;
        self.data.truncate(self.current_base);
        if self.data.is_empty() {
            self.current_base = 0;
            self.current_size = 0;
        } else if self.current_base >= prev_base {
            self.current_size = self.current_base - prev_base;
            self.current_base = prev_base;
        } else {
            self.current_base = 0;
            self.current_size = self.data.len();
        }
        Ok(prev_pc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_entry_and_access() {
        let mut stack = FrameStack::new();
        stack.push_entry(32, -1);
        let off = stack.current_data_offset();
        // Data area should be 32 bytes of zeroes
        assert_eq!(stack.data.len() - off, 32);
        assert!(stack.data[off..].iter().all(|&b| b == 0));
    }

    #[test]
    fn write_to_frame() {
        let mut stack = FrameStack::new();
        stack.push_entry(16, -1);
        let off = stack.current_data_offset();
        memory::write_word(&mut stack.data, off, 42);
        assert_eq!(memory::read_word(&stack.data, off), 42);
    }

    #[test]
    fn alloc_activate_pop() {
        let mut stack = FrameStack::new();
        stack.push_entry(16, -1);
        let off0 = stack.current_data_offset();
        memory::write_word(&mut stack.data, off0, 100);

        // Allocate and activate a new frame
        let pending = stack.alloc_pending(8).unwrap();
        stack.activate_pending(pending, 5).unwrap();
        let off1 = stack.current_data_offset();
        assert_eq!(stack.data.len() - off1, 8);

        // Write to new frame
        memory::write_word(&mut stack.data, off1, 200);
        assert_eq!(memory::read_word(&stack.data, off1), 200);

        // Pop back to entry frame
        let saved_pc = stack.pop().unwrap();
        assert_eq!(saved_pc, 5);
        let off_restored = stack.current_data_offset();
        assert_eq!(stack.data.len() - off_restored, 16);
        assert_eq!(memory::read_word(&stack.data, off_restored), 100);
    }

    #[test]
    fn pop_entry_returns_sentinel() {
        let mut stack = FrameStack::new();
        stack.push_entry(8, -1);
        let pc = stack.pop().unwrap();
        assert_eq!(pc, -1);
    }

    #[test]
    fn activate_pending_rejects_out_of_range_offset() {
        let mut stack = FrameStack::new();
        stack.push_entry(16, -1);
        // `call` takes its frame pointer straight from a bytecode operand, so a
        // bogus value must be reported as an error rather than panicking.
        let result = stack.activate_pending(0x1000_0000, 5);
        assert!(
            result.is_err(),
            "out-of-range frame pointer must be rejected"
        );
    }

    #[test]
    fn activate_pending_rejects_offset_inside_header() {
        let mut stack = FrameStack::new();
        stack.push_entry(16, -1);
        // A data area offset below the header size cannot name a real frame.
        assert!(stack.activate_pending(4, 5).is_err());
    }

    #[test]
    fn alloc_pending_reserves_distinct_frames() {
        let mut stack = FrameStack::new();
        stack.push_entry(16, -1);

        // `frame T1; frame T2` before any `call` must yield two disjoint frames.
        let first = stack.alloc_pending(8).unwrap();
        memory::write_word(&mut stack.data, first, 111);
        let second = stack.alloc_pending(8).unwrap();
        memory::write_word(&mut stack.data, second, 222);

        assert_ne!(first, second, "pending frames must not share a base");
        assert!(second >= first + 8, "pending frames must not overlap");
        assert_eq!(
            memory::read_word(&stack.data, first),
            111,
            "second alloc must not clobber the first pending frame"
        );
        assert_eq!(memory::read_word(&stack.data, second), 222);
    }

    #[test]
    fn alloc_pending_rejects_oversized_frame() {
        let mut stack = FrameStack::new();
        stack.push_entry(16, -1);
        assert!(
            stack.alloc_pending(usize::MAX / 2).is_err(),
            "absurd frame sizes must be rejected, not allocated"
        );
    }

    #[test]
    fn alloc_pending_stays_inside_the_frame_address_range() {
        // Frame offsets are encoded as virtual addresses below MP_BASE, so the
        // stack must never grow into the module MP address range.
        let mut stack = FrameStack::new();
        stack.push_entry(16, -1);
        assert!(stack.alloc_pending(MAX_FRAME_STACK_SIZE).is_err());
        assert!(stack.data.len() < crate::address::MP_BASE);
    }
}
