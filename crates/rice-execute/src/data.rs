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

    for item in items {
        match item {
            DataItem::Bytes { offset, values } => {
                let off = *offset as usize;
                for (i, &b) in values.iter().enumerate() {
                    if off + i < mp.len() {
                        memory::write_byte(&mut mp, off + i, b);
                    }
                }
            }
            DataItem::Words { offset, values } => {
                let off = *offset as usize;
                for (i, &w) in values.iter().enumerate() {
                    let pos = off + i * 4;
                    if pos + 4 <= mp.len() {
                        memory::write_word(&mut mp, pos, w);
                    }
                }
            }
            DataItem::Bigs { offset, values } => {
                let off = *offset as usize;
                for (i, &b) in values.iter().enumerate() {
                    let pos = off + i * 8;
                    if pos + 8 <= mp.len() {
                        memory::write_big(&mut mp, pos, b);
                    }
                }
            }
            DataItem::Reals { offset, values } => {
                let off = *offset as usize;
                for (i, &r) in values.iter().enumerate() {
                    let pos = off + i * 8;
                    if pos + 8 <= mp.len() {
                        memory::write_real(&mut mp, pos, r);
                    }
                }
            }
            DataItem::String { offset, value } => {
                let off = *offset as usize;
                // Allocate string on the heap and store HeapId in MP.
                let id = heap.alloc(0, HeapData::Str(value.clone()));
                if off + 4 <= mp.len() {
                    memory::write_word(&mut mp, off, id as i32);
                }
            }
            // Array, SetArray, RestoreBase: no-op for now (no heap arrays in data init)
            DataItem::Array { .. } | DataItem::SetArray { .. } | DataItem::RestoreBase => {}
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
