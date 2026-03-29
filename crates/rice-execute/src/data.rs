//! Module data (MP) initialization from DataItem entries.

use ricevm_core::DataItem;

use crate::memory;

/// Initialize the module data pointer (MP) memory from DataItem entries.
///
/// Returns a flat byte buffer of size `data_size`, with values written
/// at their specified offsets.
pub(crate) fn init_mp(data_size: usize, items: &[DataItem]) -> Vec<u8> {
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
                let bytes = value.as_bytes();
                for (i, &b) in bytes.iter().enumerate() {
                    if off + i < mp.len() {
                        memory::write_byte(&mut mp, off + i, b);
                    }
                }
            }
            // Array, SetArray, RestoreBase: no-op for milestone 1 (no heap)
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
        let items = vec![DataItem::Words {
            offset: 0,
            values: vec![42, -1],
        }];
        let mp = init_mp(8, &items);
        assert_eq!(memory::read_word(&mp, 0), 42);
        assert_eq!(memory::read_word(&mp, 4), -1);
    }

    #[test]
    fn init_mp_bytes_and_string() {
        let items = vec![
            DataItem::Bytes {
                offset: 0,
                values: vec![1, 2, 3],
            },
            DataItem::String {
                offset: 4,
                value: "hi".to_string(),
            },
        ];
        let mp = init_mp(8, &items);
        assert_eq!(mp[0], 1);
        assert_eq!(mp[1], 2);
        assert_eq!(mp[2], 3);
        assert_eq!(mp[4], b'h');
        assert_eq!(mp[5], b'i');
    }

    #[test]
    fn init_mp_empty() {
        let mp = init_mp(16, &[]);
        assert_eq!(mp.len(), 16);
        assert!(mp.iter().all(|&b| b == 0));
    }
}
