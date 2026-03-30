//! Module data (MP) initialization from DataItem entries.

use ricevm_core::DataItem;

use crate::heap::{Heap, HeapData};
use crate::memory;

/// Initialize the module data pointer (MP) memory from DataItem entries.
///
/// Returns a flat byte buffer of size `data_size`, with values written
/// at their specified offsets. Strings are allocated on the heap and
/// their HeapId is stored as a Word at the offset.
pub(crate) fn init_mp(data_size: usize, items: &[DataItem], heap: &mut Heap) -> Vec<u8> {
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
                element_type: _,
                length,
            } => {
                let off = *offset as usize;
                let len = *length as usize;
                // Element size defaults to 4 (word-sized); the type descriptor
                // would give a more precise size, but we don't have it here.
                let elem_size = 4;
                let data = vec![0u8; len * elem_size];
                let arr_id = heap.alloc(
                    0,
                    HeapData::Array {
                        elem_type: 0,
                        elem_size,
                        data,
                        length: len,
                    },
                );
                if off + 4 <= mp.len() {
                    memory::write_word(&mut mp, off, arr_id as i32);
                }
            }
            DataItem::SetArray { offset, index } => {
                let off = *offset as usize;
                let idx = *index as usize;
                // Read the array HeapId from MP at the given offset.
                let arr_id = if off + 4 <= mp.len() {
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
    if let Some(&(arr_id, _, _)) = array_stack.last() {
        if let Some(obj) = heap.get_mut(arr_id) {
            if let HeapData::Array { data, .. } = &mut obj.data {
                return data;
            }
        }
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
}
