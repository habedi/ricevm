//! Module data (MP) initialization from DataItem entries.

use ricevm_core::DataItem;

use crate::heap::{Heap, HeapData};
use crate::memory;
// Arrays built from module data are bounded by the same limit as arrays
// allocated at run time by `newa`. Sharing the definition is the point: a
// module must not be able to obtain through its data section an array that
// `newa` would refuse it.
use crate::ops::heap::MAX_ARRAY_BYTES;

/// Initialize the module data pointer (MP) memory from DataItem entries.
///
/// Returns a flat byte buffer of size `data_size`, with values written
/// at their specified offsets. Strings are allocated on the heap and
/// their HeapId is stored as a Word at the offset.
#[allow(dead_code)]
pub(crate) fn init_mp(data_size: usize, items: &[DataItem], heap: &mut Heap) -> Vec<u8> {
    init_mp_with_types(data_size, items, heap, &[])
}

pub(crate) fn init_mp_with_types(
    data_size: usize,
    items: &[DataItem],
    heap: &mut Heap,
    types: &[ricevm_core::TypeDescriptor],
) -> Vec<u8> {
    let mut mp = vec![0u8; data_size];

    // Stack for tracking nested array initialization.
    // Each entry is (array_heap_id, elem_size, base_mp_offset).
    let mut array_stack: Vec<(u32, usize, usize)> = Vec::new();

    for item in items {
        match item {
            DataItem::Bytes { offset, values } => {
                let Some(off) = checked_offset(*offset) else {
                    continue;
                };
                let buf = active_buffer(&mut mp, heap, &array_stack);
                for (i, &b) in values.iter().enumerate() {
                    match off.checked_add(i) {
                        Some(pos) if pos < buf.len() => memory::write_byte(buf, pos, b),
                        _ => break,
                    }
                }
            }
            DataItem::Words { offset, values } => {
                let Some(off) = checked_offset(*offset) else {
                    continue;
                };
                let buf = active_buffer(&mut mp, heap, &array_stack);
                for (i, &w) in values.iter().enumerate() {
                    match element_pos(off, i, 4) {
                        Some(pos) if pos + 4 <= buf.len() => memory::write_word(buf, pos, w),
                        _ => break,
                    }
                }
            }
            DataItem::Bigs { offset, values } => {
                let Some(off) = checked_offset(*offset) else {
                    continue;
                };
                let buf = active_buffer(&mut mp, heap, &array_stack);
                for (i, &b) in values.iter().enumerate() {
                    match element_pos(off, i, 8) {
                        Some(pos) if pos + 8 <= buf.len() => memory::write_big(buf, pos, b),
                        _ => break,
                    }
                }
            }
            DataItem::Reals { offset, values } => {
                let Some(off) = checked_offset(*offset) else {
                    continue;
                };
                let buf = active_buffer(&mut mp, heap, &array_stack);
                for (i, &r) in values.iter().enumerate() {
                    match element_pos(off, i, 8) {
                        Some(pos) if pos + 8 <= buf.len() => memory::write_real(buf, pos, r),
                        _ => break,
                    }
                }
            }
            DataItem::String { offset, value } => {
                let Some(off) = checked_offset(*offset) else {
                    continue;
                };
                // Check the destination slot *before* allocating. An object
                // whose id has nowhere to be stored is referenced by nothing,
                // yet is born with ref_count 1 and so survives every GC pass
                // for the VM's lifetime.
                if !word_slot_fits(&mut mp, heap, &array_stack, off) {
                    continue;
                }
                let id = heap.alloc(0, HeapData::Str(value.clone()));
                let buf = active_buffer(&mut mp, heap, &array_stack);
                memory::write_word(buf, off, id as i32);
            }
            DataItem::Array {
                offset,
                element_type,
                length,
            } => {
                let Some(off) = checked_offset(*offset) else {
                    continue;
                };
                let Ok(len) = usize::try_from(*length) else {
                    continue;
                };
                // As for String: an array id with nowhere to go would pin the
                // array on the heap forever, so validate the slot first.
                if !word_slot_fits(&mut mp, heap, &array_stack, off) {
                    continue;
                }
                let et = *element_type as usize;
                // Look up element size from type descriptors; default to 4.
                let elem_size = types.get(et).map(|td| td.size as usize).unwrap_or(4).max(1);
                // Refuse hostile sizes rather than aborting on allocation.
                let byte_len = match len.checked_mul(elem_size) {
                    Some(n) if n <= MAX_ARRAY_BYTES => n,
                    _ => continue,
                };
                let data = vec![0u8; byte_len];
                let arr_id = heap.alloc(
                    et as u32,
                    HeapData::Array {
                        elem_type: et as u32,
                        elem_size,
                        data,
                        length: len,
                    },
                );
                let buf = active_buffer(&mut mp, heap, &array_stack);
                memory::write_word(buf, off, arr_id as i32);
            }
            DataItem::SetArray { offset, index } => {
                let Some(off) = checked_offset(*offset) else {
                    continue;
                };
                let Ok(idx) = usize::try_from(*index) else {
                    continue;
                };
                // Read the array HeapId from the active buffer (MP or parent array).
                // When inside a nested array context (array_stack non-empty),
                // the offset refers to the current array's data, not MP.
                let arr_id = if let Some(&(parent_arr_id, _, _)) = array_stack.last() {
                    // Inside array context: read from the array's heap data
                    if let Some(obj) = heap.get(parent_arr_id) {
                        match &obj.data {
                            HeapData::Array { data, .. } if off + 4 <= data.len() => {
                                memory::read_word(data, off) as u32
                            }
                            _ => 0,
                        }
                    } else {
                        0
                    }
                } else if off + 4 <= mp.len() {
                    memory::read_word(&mp, off) as u32
                } else {
                    0
                };
                if arr_id != 0 {
                    let elem_size = if let Some(obj) = heap.get(arr_id) {
                        match &obj.data {
                            HeapData::Array { elem_size, .. } => *elem_size,
                            _ => 4,
                        }
                    } else {
                        4
                    };
                    let Some(base) = idx.checked_mul(elem_size) else {
                        continue;
                    };
                    array_stack.push((arr_id, elem_size, base));
                }
            }
            DataItem::RestoreBase => {
                array_stack.pop();
            }
        }
    }

    mp
}

/// Convert an untrusted module data offset to a buffer index.
/// Negative offsets are rejected: they can never name a valid location.
fn checked_offset(offset: ricevm_core::Word) -> Option<usize> {
    usize::try_from(offset).ok()
}

/// Byte position of element `index` of `size` bytes at `base`, if it does not overflow.
fn element_pos(base: usize, index: usize, size: usize) -> Option<usize> {
    index.checked_mul(size).and_then(|d| base.checked_add(d))
}

/// Whether a 4-byte word written at `off` would land inside the buffer that
/// is currently active. Callers that allocate a heap object and store its id
/// must ask this *first*: an object whose id cannot be stored is unreachable
/// but never freed.
fn word_slot_fits(
    mp: &mut Vec<u8>,
    heap: &mut Heap,
    array_stack: &[(u32, usize, usize)],
    off: usize,
) -> bool {
    match off.checked_add(4) {
        Some(end) => end <= active_buffer(mp, heap, array_stack).len(),
        None => false,
    }
}

/// Get the active write buffer: either an array element's data or the MP.
fn active_buffer<'a>(
    mp: &'a mut Vec<u8>,
    heap: &'a mut Heap,
    array_stack: &[(u32, usize, usize)],
) -> &'a mut Vec<u8> {
    if let Some(&(arr_id, _, _)) = array_stack.last()
        && let Some(obj) = heap.get_mut(arr_id)
        && let HeapData::Array { data, .. } = &mut obj.data
    {
        return data;
    }
    mp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_mp_words() {
        let mut heap = Heap::new();
        let items = vec![DataItem::Words {
            offset: 0,
            values: vec![42, -1],
        }];
        let mp = init_mp(8, &items, &mut heap);
        assert_eq!(memory::read_word(&mp, 0), 42);
        assert_eq!(memory::read_word(&mp, 4), -1);
    }

    #[test]
    fn init_mp_string_on_heap() {
        let mut heap = Heap::new();
        let items = vec![DataItem::String {
            offset: 0,
            value: "hello".to_string(),
        }];
        let mp = init_mp(8, &items, &mut heap);
        let id = memory::read_word(&mp, 0) as u32;
        assert_ne!(id, 0);
        assert_eq!(heap.get_string(id), Some("hello"));
    }

    #[test]
    fn init_mp_empty() {
        let mut heap = Heap::new();
        let mp = init_mp(16, &[], &mut heap);
        assert_eq!(mp.len(), 16);
        assert!(mp.iter().all(|&b| b == 0));
    }

    #[test]
    fn array_element_size_from_type_descriptor() {
        let mut heap = Heap::new();
        let types = vec![
            ricevm_core::TypeDescriptor {
                id: 0,
                size: 4,
                pointer_map: ricevm_core::PointerMap { bytes: vec![] },
                pointer_count: 0,
            },
            // Placeholder types 1..5
            ricevm_core::TypeDescriptor {
                id: 1,
                size: 4,
                pointer_map: ricevm_core::PointerMap { bytes: vec![] },
                pointer_count: 0,
            },
            ricevm_core::TypeDescriptor {
                id: 2,
                size: 4,
                pointer_map: ricevm_core::PointerMap { bytes: vec![] },
                pointer_count: 0,
            },
            ricevm_core::TypeDescriptor {
                id: 3,
                size: 4,
                pointer_map: ricevm_core::PointerMap { bytes: vec![] },
                pointer_count: 0,
            },
            ricevm_core::TypeDescriptor {
                id: 4,
                size: 4,
                pointer_map: ricevm_core::PointerMap { bytes: vec![] },
                pointer_count: 0,
            },
            ricevm_core::TypeDescriptor {
                id: 5,
                size: 4,
                pointer_map: ricevm_core::PointerMap { bytes: vec![] },
                pointer_count: 0,
            },
            // Type 6 with size 8
            ricevm_core::TypeDescriptor {
                id: 6,
                size: 8,
                pointer_map: ricevm_core::PointerMap { bytes: vec![] },
                pointer_count: 0,
            },
        ];
        let items = vec![DataItem::Array {
            offset: 0,
            element_type: 6,
            length: 3,
        }];
        let mp = init_mp_with_types(8, &items, &mut heap, &types);
        let arr_id = memory::read_word(&mp, 0) as u32;
        let obj = heap.get(arr_id).expect("array should exist");
        match &obj.data {
            HeapData::Array {
                elem_size,
                data,
                length,
                ..
            } => {
                assert_eq!(*elem_size, 8, "elem_size should come from types[6].size");
                assert_eq!(*length, 3);
                assert_eq!(data.len(), 3 * 8);
            }
            _ => panic!("expected Array"),
        }
    }

    #[test]
    fn array_with_negative_length_is_ignored() {
        let mut heap = Heap::new();
        let items = vec![DataItem::Array {
            offset: 0,
            element_type: 0,
            length: -1,
        }];
        let mp = init_mp(8, &items, &mut heap);
        assert_eq!(
            memory::read_word(&mp, 0),
            0,
            "a negative array length must not allocate"
        );
    }

    #[test]
    fn array_with_oversized_length_is_ignored() {
        let mut heap = Heap::new();
        let types = vec![ricevm_core::TypeDescriptor {
            id: 0,
            size: 8,
            pointer_map: ricevm_core::PointerMap { bytes: vec![] },
            pointer_count: 0,
        }];
        let items = vec![DataItem::Array {
            offset: 0,
            element_type: 0,
            length: i32::MAX,
        }];
        let mp = init_mp_with_types(8, &items, &mut heap, &types);
        assert_eq!(
            memory::read_word(&mp, 0),
            0,
            "an oversized array must not allocate"
        );
    }

    #[test]
    fn negative_offsets_are_ignored() {
        let mut heap = Heap::new();
        let items = vec![
            DataItem::Bytes {
                offset: -1,
                values: vec![1, 2, 3],
            },
            DataItem::Words {
                offset: -4,
                values: vec![7, 8],
            },
            DataItem::Bigs {
                offset: -8,
                values: vec![9],
            },
            DataItem::Reals {
                offset: -8,
                values: vec![1.0],
            },
            DataItem::String {
                offset: -4,
                value: "x".to_string(),
            },
            DataItem::Array {
                offset: -4,
                element_type: 0,
                length: 2,
            },
        ];
        let mp = init_mp(16, &items, &mut heap);
        assert!(
            mp.iter().all(|&b| b == 0),
            "negative offsets must not write into MP"
        );
    }

    #[test]
    fn string_with_unwritable_offset_does_not_allocate() {
        let mut heap = Heap::new();
        // Non-negative (so `checked_offset` accepts it) but the 4-byte id
        // does not fit in the 8-byte MP buffer. Allocating first would leave
        // a live, unreferenced string pinned on the heap for the VM's life.
        let items = vec![DataItem::String {
            offset: 6,
            value: "unreachable".to_string(),
        }];
        let mp = init_mp(8, &items, &mut heap);
        assert!(
            mp.iter().all(|&b| b == 0),
            "an out-of-range offset must not write into MP"
        );
        assert_eq!(
            heap.len(),
            0,
            "a string whose id has nowhere to go must not be allocated"
        );
    }

    #[test]
    fn array_with_unwritable_offset_does_not_allocate() {
        let mut heap = Heap::new();
        let items = vec![DataItem::Array {
            offset: 6,
            element_type: 0,
            length: 4,
        }];
        let mp = init_mp(8, &items, &mut heap);
        assert!(
            mp.iter().all(|&b| b == 0),
            "an out-of-range offset must not write into MP"
        );
        assert_eq!(
            heap.len(),
            0,
            "an array whose id has nowhere to go must not be allocated"
        );
    }

    #[test]
    fn array_one_element_over_the_cap_is_refused() {
        // The counterpart of `op_newa_respects_the_same_cap_as_the_data_section`
        // in ops/heap.rs: both paths must refuse the same request, so a module
        // cannot get through its data section what `newa` denies it, or the
        // other way round.
        let mut heap = Heap::new();
        let types = vec![ricevm_core::TypeDescriptor {
            id: 0,
            size: 16,
            pointer_map: ricevm_core::PointerMap { bytes: vec![] },
            pointer_count: 0,
        }];
        let over_cap = MAX_ARRAY_BYTES / 16 + 1;
        let items = vec![DataItem::Array {
            offset: 0,
            element_type: 0,
            length: i32::try_from(over_cap).expect("length fits in a Word"),
        }];
        let mp = init_mp_with_types(8, &items, &mut heap, &types);
        assert_eq!(memory::read_word(&mp, 0), 0);
        assert_eq!(heap.len(), 0, "an array over the cap must not be allocated");
    }

    #[test]
    fn nested_array_initialization() {
        let mut heap = Heap::new();
        // Outer array at MP offset 0 with 2 elements (elem_size=4, each element is a pointer)
        // Inner data written via SetArray into the outer array's heap data
        let items = vec![
            DataItem::Array {
                offset: 0,
                element_type: 0, // will default to elem_size=4
                length: 2,
            },
            // Create inner array at MP offset 4
            DataItem::Array {
                offset: 4,
                element_type: 0,
                length: 3,
            },
            // SetArray: set context to outer array (at MP offset 0), element index 0
            DataItem::SetArray {
                offset: 0,
                index: 0,
            },
            // Write a word into the outer array's element 0 data
            DataItem::Words {
                offset: 0,
                values: vec![42],
            },
            DataItem::RestoreBase,
        ];
        let mp = init_mp_with_types(8, &items, &mut heap, &[]);

        // The outer array should have been allocated on the heap
        let outer_id = memory::read_word(&mp, 0) as u32;
        let obj = heap.get(outer_id).expect("outer array should exist");
        match &obj.data {
            HeapData::Array { data, .. } => {
                // The word 42 should have been written to the array data at offset 0
                assert_eq!(memory::read_word(data, 0), 42);
            }
            _ => panic!("expected Array"),
        }
    }
}
