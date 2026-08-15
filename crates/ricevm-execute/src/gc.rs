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
/// 3. The heap reference table (`indx`/`lea` element references), which holds
///    real HeapIds that never appear as words in any scanned buffer
///
/// Then sweeps all unmarked objects from the heap.
pub(crate) fn collect(
    heap: &mut Heap,
    frames: &FrameStack,
    mp: &[u8],
    loaded_modules: &[crate::vm::LoadedModule],
    suspended_threads: &std::collections::VecDeque<crate::vm::SuspendedThread>,
    caller_mp_stack: &[(usize, Vec<u8>)],
    heap_refs: &[(HeapId, usize)],
) {
    if heap.len() == 0 {
        return;
    }

    let mut marked = HashSet::new();

    // Mark phase: scan current thread's frame stack
    scan_buffer(&frames.data, heap, &mut marked);

    // Mark phase: scan current module data
    scan_buffer(mp, heap, &mut marked);

    // Mark phase: scan current thread's caller MP stack (active during loaded module calls)
    for (_, caller_mp) in caller_mp_stack {
        scan_buffer(caller_mp, heap, &mut marked);
    }

    // Mark phase: scan all loaded modules' MPs
    for lm in loaded_modules {
        scan_buffer(&lm.mp, heap, &mut marked);
    }

    // Mark phase: the current thread's heap reference table. `indx`/`lea` store
    // a flagged index in the frame, so these ids are only reachable from here.
    scan_heap_refs(heap_refs, heap, &mut marked);

    // Mark phase: scan suspended threads' frames, MPs, caller stacks, and refs
    for thread in suspended_threads {
        scan_buffer(&thread.frames.data, heap, &mut marked);
        scan_buffer(&thread.mp, heap, &mut marked);
        for (_, caller_mp) in &thread.caller_mp_stack {
            scan_buffer(caller_mp, heap, &mut marked);
        }
        scan_heap_refs(&thread.heap_refs, heap, &mut marked);
    }

    // Sweep phase: remove all unmarked objects
    heap.sweep(&marked);
}

/// Scan a byte buffer for potential heap references (word-aligned HeapIds)
/// and mark everything reachable from them.
fn scan_buffer(buf: &[u8], heap: &Heap, marked: &mut HashSet<HeapId>) {
    let mut worklist = Vec::new();
    collect_ids(buf, heap, &mut worklist);
    mark_all(worklist, heap, marked);
}

/// Mark every object named by a heap reference table entry.
fn scan_heap_refs(refs: &[(HeapId, usize)], heap: &Heap, marked: &mut HashSet<HeapId>) {
    let worklist = refs.iter().map(|&(id, _)| id).collect();
    mark_all(worklist, heap, marked);
}

/// Push every word-aligned value in `buf` that names a live heap object.
fn collect_ids(buf: &[u8], heap: &Heap, out: &mut Vec<HeapId>) {
    let mut offset = 0;
    while offset + 4 <= buf.len() {
        let word = memory::read_word(buf, offset) as u32;
        if word != NIL && heap.contains(word) {
            out.push(word);
        }
        offset += 4;
    }
}

/// Mark everything reachable from `worklist`.
///
/// Iterative, with the worklist on the heap: a guest list is as long as the
/// guest makes it, and the periodic collection (vm.rs) can fire while one is
/// live. Recursing per list node or per record field would overflow the native
/// stack, which aborts the process rather than raising a catchable error.
fn mark_all(mut worklist: Vec<HeapId>, heap: &Heap, marked: &mut HashSet<HeapId>) {
    while let Some(id) = worklist.pop() {
        if id == NIL || !marked.insert(id) {
            continue;
        }
        let Some(obj) = heap.get(id) else { continue };
        match &obj.data {
            HeapData::Record(data) | HeapData::Array { data, .. } | HeapData::Adt { data, .. } => {
                collect_ids(data, heap, &mut worklist);
            }
            HeapData::List { head, tail } => {
                collect_ids(head, heap, &mut worklist);
                worklist.push(*tail);
            }
            HeapData::ArraySlice { parent_id, .. } => {
                worklist.push(*parent_id);
            }
            // A value in transit through a channel is only referenced by the
            // channel's payload buffer; scan it so it is not swept.
            HeapData::Channel { pending, .. } => {
                if let Some(pending) = pending {
                    collect_ids(pending, heap, &mut worklist);
                }
            }
            HeapData::Str(_)
            | HeapData::ModuleRef { .. }
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
        collect(
            &mut heap,
            &frames,
            &[],
            &[],
            &std::collections::VecDeque::new(),
            &[],
            &[],
        );

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

        collect(
            &mut heap,
            &frames,
            &[],
            &[],
            &std::collections::VecDeque::new(),
            &[],
            &[],
        );

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

        collect(
            &mut heap,
            &frames,
            &[],
            &[],
            &std::collections::VecDeque::new(),
            &[],
            &[],
        );

        // Both the list node and the string it references should survive
        assert!(heap.get(list_id).is_some());
        assert!(heap.get(str_id).is_some());
    }

    #[test]
    fn gc_preserves_suspended_thread_references() {
        let mut heap = Heap::new();
        let id_in_thread = heap.alloc(0, HeapData::Str("thread-owned".to_string()));
        let id_unreachable = heap.alloc(0, HeapData::Str("orphan".to_string()));

        // Current thread has no references
        let frames = FrameStack::new();

        // Suspended thread holds id_in_thread in its frame
        let mut thread_frames = FrameStack::new();
        thread_frames.push_entry(16, -1);
        let off = thread_frames.current_data_offset();
        memory::write_word(&mut thread_frames.data, off, id_in_thread as i32);

        let mut thread_queue = std::collections::VecDeque::new();
        thread_queue.push_back(crate::vm::SuspendedThread {
            frames: thread_frames,
            mp: Vec::new(),
            pc: 0,
            heap_refs: Vec::new(),
            last_error: String::new(),
            current_loaded_module: None,
            caller_mp_stack: Vec::new(),
            blocked_on: None,
        });

        collect(&mut heap, &frames, &[], &[], &thread_queue, &[], &[]);

        // Object referenced by the suspended thread must survive
        assert!(
            heap.get(id_in_thread).is_some(),
            "GC must not collect objects referenced by suspended threads"
        );
        // Unreachable object should be collected
        assert!(
            heap.get(id_unreachable).is_none(),
            "GC should collect unreachable objects"
        );
    }

    #[test]
    fn gc_preserves_array_and_elements() {
        let mut heap = Heap::new();
        let str_id = heap.alloc(0, HeapData::Str("element".to_string()));
        let mut arr_data = vec![0u8; 4];
        memory::write_word(&mut arr_data, 0, str_id as i32);
        let arr_id = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: arr_data,
                length: 1,
            },
        );

        let mut frames = FrameStack::new();
        frames.push_entry(16, -1);
        let off = frames.current_data_offset();
        memory::write_word(&mut frames.data, off, arr_id as i32);

        collect(
            &mut heap,
            &frames,
            &[],
            &[],
            &std::collections::VecDeque::new(),
            &[],
            &[],
        );

        assert!(heap.get(arr_id).is_some(), "array should survive GC");
        assert!(
            heap.get(str_id).is_some(),
            "array element should survive GC"
        );
    }

    #[test]
    fn gc_collects_multiple_unreachable() {
        let mut heap = Heap::new();
        let ids: Vec<u32> = (0..5)
            .map(|i| heap.alloc(0, HeapData::Str(format!("orphan-{i}"))))
            .collect();

        let frames = FrameStack::new();
        collect(
            &mut heap,
            &frames,
            &[],
            &[],
            &std::collections::VecDeque::new(),
            &[],
            &[],
        );

        for id in ids {
            assert!(
                heap.get(id).is_none(),
                "unreachable object {id} should be collected"
            );
        }
    }

    #[test]
    fn gc_preserves_heap_ref_table_entries() {
        let mut heap = Heap::new();
        let arr_id = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: vec![0u8; 8],
                length: 2,
            },
        );

        // `indx`/`lea` leave a flagged index in the frame and the real id in
        // heap_refs, so the frame scan alone can never find this object.
        let frames = FrameStack::new();
        let heap_refs = vec![(arr_id, 4usize)];

        collect(
            &mut heap,
            &frames,
            &[],
            &[],
            &std::collections::VecDeque::new(),
            &[],
            &heap_refs,
        );

        assert!(
            heap.get(arr_id).is_some(),
            "objects referenced by the heap_refs table must survive GC"
        );
    }

    #[test]
    fn gc_preserves_suspended_thread_heap_refs() {
        let mut heap = Heap::new();
        let str_id = heap.alloc(0, HeapData::Str("element-ref".to_string()));

        let mut thread_queue = std::collections::VecDeque::new();
        thread_queue.push_back(crate::vm::SuspendedThread {
            frames: FrameStack::new(),
            mp: Vec::new(),
            pc: 0,
            heap_refs: vec![(str_id, 0)],
            last_error: String::new(),
            current_loaded_module: None,
            caller_mp_stack: Vec::new(),
            blocked_on: None,
        });

        collect(
            &mut heap,
            &FrameStack::new(),
            &[],
            &[],
            &thread_queue,
            &[],
            &[],
        );

        assert!(
            heap.get(str_id).is_some(),
            "a suspended thread's heap_refs must be GC roots"
        );
    }

    #[test]
    fn gc_preserves_channel_payload() {
        let mut heap = Heap::new();
        let str_id = heap.alloc(0, HeapData::Str("in transit".to_string()));
        let mut pending = vec![0u8; 4];
        memory::write_word(&mut pending, 0, str_id as i32);
        let chan_id = heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: Some(pending),
            },
        );

        let mut frames = FrameStack::new();
        frames.push_entry(16, -1);
        let off = frames.current_data_offset();
        memory::write_word(&mut frames.data, off, chan_id as i32);

        collect(
            &mut heap,
            &frames,
            &[],
            &[],
            &std::collections::VecDeque::new(),
            &[],
            &[],
        );

        assert!(heap.get(chan_id).is_some(), "channel should survive GC");
        assert!(
            heap.get(str_id).is_some(),
            "a value in transit through a channel must survive GC"
        );
    }

    /// A guest list is as long as the guest cares to make it, and the periodic
    /// collection (vm.rs, every 10,000 instructions) can fire while one is
    /// live. Marking must not recurse per node: a native stack overflow aborts
    /// the process instead of surfacing as a catchable error.
    ///
    /// The chain is marked on a thread with a deliberately small stack so the
    /// test does not depend on the harness's stack size.
    #[test]
    fn gc_marks_a_long_list_without_overflowing_the_stack() {
        const NODES: usize = 100_000;

        std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(|| {
                let mut heap = Heap::new();
                let mut tail = NIL;
                for _ in 0..NODES {
                    tail = heap.alloc(
                        0,
                        HeapData::List {
                            head: vec![0u8; 4],
                            tail,
                        },
                    );
                }

                let mut frames = FrameStack::new();
                frames.push_entry(16, -1);
                let off = frames.current_data_offset();
                memory::write_word(&mut frames.data, off, tail as i32);

                collect(
                    &mut heap,
                    &frames,
                    &[],
                    &[],
                    &std::collections::VecDeque::new(),
                    &[],
                    &[],
                );

                assert_eq!(
                    heap.len(),
                    NODES,
                    "every node of a rooted list must be marked and survive"
                );
            })
            .expect("spawn marking thread")
            .join()
            .expect("marking a long list must not overflow the stack");
    }

    #[test]
    fn gc_preserves_mp_references() {
        let mut heap = Heap::new();
        let str_id = heap.alloc(0, HeapData::Str("mp-owned".to_string()));

        let frames = FrameStack::new();
        let mut mp = vec![0u8; 8];
        memory::write_word(&mut mp, 0, str_id as i32);

        collect(
            &mut heap,
            &frames,
            &mp,
            &[],
            &std::collections::VecDeque::new(),
            &[],
            &[],
        );

        assert!(
            heap.get(str_id).is_some(),
            "MP-referenced object should survive GC"
        );
    }
}
