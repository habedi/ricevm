//! Mark-and-sweep garbage collector for cyclic reference detection.
//!
//! Reference counting handles most cases, but cyclic references
//! (e.g., A -> B -> A) will never reach ref count 0. The mark-and-sweep
//! collector runs periodically to find and free unreachable cycles.

use std::collections::HashSet;

use crate::frame::FrameStack;
use crate::heap::{Heap, HeapData, HeapId, NIL};
use crate::memory;

/// Run a mark-and-sweep garbage collection pass.
///
/// Marks all reachable objects starting from:
/// 1. The frame stack (scanning for pointer-sized words that look like HeapIds)
/// 2. The module data (MP)
///
/// Then sweeps all unmarked objects from the heap.
pub(crate) fn collect(
    heap: &mut Heap,
    frames: &FrameStack,
    mp: &[u8],
    loaded_modules: &[crate::vm::LoadedModule],
) {
    if heap.len() == 0 {
        return;
    }

    let mut marked = HashSet::new();

    // Mark phase: scan frame stack for potential heap references
    scan_buffer(&frames.data, heap, &mut marked);

    // Mark phase: scan module data
    scan_buffer(mp, heap, &mut marked);

    // Mark phase: scan all loaded modules' MPs
    for lm in loaded_modules {
        scan_buffer(&lm.mp, heap, &mut marked);
    }

    // Sweep phase: remove all unmarked objects
    heap.sweep(&marked);
}

/// Scan a byte buffer for potential heap references (word-aligned HeapIds).
fn scan_buffer(buf: &[u8], heap: &Heap, marked: &mut HashSet<HeapId>) {
    // Scan every word-aligned position for potential HeapIds
    let mut offset = 0;
    while offset + 4 <= buf.len() {
        let word = memory::read_word(buf, offset) as u32;
        if word != NIL && heap.contains(word) && !marked.contains(&word) {
            mark_object(word, heap, marked);
        }
        offset += 4;
    }
}

/// Recursively mark an object and everything it references.
fn mark_object(id: HeapId, heap: &Heap, marked: &mut HashSet<HeapId>) {
    if id == NIL || marked.contains(&id) {
        return;
    }
    marked.insert(id);

    // Scan the object's data for more heap references
    if let Some(obj) = heap.get(id) {
        match &obj.data {
            HeapData::Record(data) | HeapData::Array { data, .. } | HeapData::Adt { data, .. } => {
                // Scan the data buffer for potential HeapIds
                let mut offset = 0;
                while offset + 4 <= data.len() {
                    let word = memory::read_word(data, offset) as u32;
                    if word != NIL && heap.contains(word) {
                        mark_object(word, heap, marked);
                    }
                    offset += 4;
                }
            }
            HeapData::List { head, tail } => {
                // Scan head buffer
                let mut offset = 0;
                while offset + 4 <= head.len() {
                    let word = memory::read_word(head, offset) as u32;
                    if word != NIL && heap.contains(word) {
                        mark_object(word, heap, marked);
                    }
                    offset += 4;
                }
                // Follow tail
                mark_object(*tail, heap, marked);
            }
            HeapData::Str(_) | HeapData::Channel { .. } => {}
            HeapData::ModuleRef { .. }
            | HeapData::MainModule { .. }
            | HeapData::LoadedModule { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameStack;

    #[test]
    fn gc_collects_unreachable() {
        let mut heap = Heap::new();
        let _id1 = heap.alloc(0, HeapData::Str("reachable".to_string()));
        let id2 = heap.alloc(0, HeapData::Str("unreachable".to_string()));

        // Only id1 is "referenced" via a frame slot
        let mut frames = FrameStack::new();
        frames.push_entry(16, -1);
        let off = frames.current_data_offset();
        memory::write_word(&mut frames.data, off, _id1 as i32);

        // Before GC: both exist
        assert!(heap.get(id2).is_some());

        // Run GC
        collect(&mut heap, &frames, &[], &[]);

        // After GC: id2 should be collected (not referenced by any root)
        assert!(heap.get(id2).is_none());
    }

    #[test]
    fn gc_preserves_reachable() {
        let mut heap = Heap::new();
        let id1 = heap.alloc(0, HeapData::Str("hello".to_string()));

        let mut frames = FrameStack::new();
        frames.push_entry(16, -1);
        let off = frames.current_data_offset();
        memory::write_word(&mut frames.data, off, id1 as i32);

        collect(&mut heap, &frames, &[], &[]);

        assert!(heap.get(id1).is_some());
        assert_eq!(heap.get_string(id1), Some("hello"));
    }

    #[test]
    fn gc_follows_list_chains() {
        let mut heap = Heap::new();
        let str_id = heap.alloc(0, HeapData::Str("tail_str".to_string()));
        let mut head = vec![0u8; 4];
        memory::write_word(&mut head, 0, str_id as i32);
        let list_id = heap.alloc(0, HeapData::List { head, tail: NIL });

        let mut frames = FrameStack::new();
        frames.push_entry(16, -1);
        let off = frames.current_data_offset();
        memory::write_word(&mut frames.data, off, list_id as i32);

        collect(&mut heap, &frames, &[], &[]);

        // Both the list node and the string it references should survive
        assert!(heap.get(list_id).is_some());
        assert!(heap.get(str_id).is_some());
    }
}
