//! Module data (MP) initialization from DataItem entries.

use ricevm_core::DataItem;

use crate::heap::{Heap, HeapData};
use crate::memory;

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
                let buf = active_buffer(&mut mp, heap, &array_stack);
                let off = *offset as usize;
                for (i, &b) in values.iter().enumerate() {
                    if off + i < buf.len() {
                        memory::write_byte(buf, off + i, b);
                    }
                }
            }
            DataItem::Words { offset, values } => {
                let buf = active_buffer(&mut mp, heap, &array_stack);
                let off = *offset as usize;
                for (i, &w) in values.iter().enumerate() {
                    let pos = off + i * 4;
                    if pos + 4 <= buf.len() {
                        memory::write_word(buf, pos, w);
                    }
                }
            }
            DataItem::Bigs { offset, values } => {
                let buf = active_buffer(&mut mp, heap, &array_stack);
                let off = *offset as usize;
                for (i, &b) in values.iter().enumerate() {
                    let pos = off + i * 8;
                    if pos + 8 <= buf.len() {
                        memory::write_big(buf, pos, b);
                    }
                }
            }
            DataItem::Reals { offset, values } => {
                let buf = active_buffer(&mut mp, heap, &array_stack);
                let off = *offset as usize;
                for (i, &r) in values.iter().enumerate() {
                    let pos = off + i * 8;
                    if pos + 8 <= buf.len() {
                        memory::write_real(buf, pos, r);
                    }
                }
            }
            DataItem::String { offset, value } => {
                let off = *offset as usize;
                let id = heap.alloc(0, HeapData::Str(value.clone()));
                let buf = active_buffer(&mut mp, heap, &array_stack);
                if off + 4 <= buf.len() {
                    memory::write_word(buf, off, id as i32);
                }
            }
            DataItem::Array {
                offset,
                element_type,
                length,
            } => {
                let off = *offset as usize;
                let len = *length as usize;
                let et = *element_type as usize;
                // Look up element size from type descriptors; default to 4.
                let elem_size = types.get(et).map(|td| td.size as usize).unwrap_or(4).max(1);
                let data = vec![0u8; len * elem_size];
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
                if off + 4 <= buf.len() {
                    memory::write_word(buf, off, arr_id as i32);
                }
            }
            DataItem::SetArray { offset, index } => {
                let off = *offset as usize;
                let idx = *index as usize;
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
                    array_stack.push((arr_id, elem_size, idx * elem_size));
                }
            }
            DataItem::RestoreBase => {
                array_stack.pop();
            }
        }
    }

    mp
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
