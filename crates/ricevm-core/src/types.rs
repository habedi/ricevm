/// Dis VM byte (unsigned 8-bit).
pub type Byte = u8;

/// Dis VM word (signed 32-bit).
pub type Word = i32;

/// Dis VM big (signed 64-bit).
pub type Big = i64;

/// Dis VM real (64-bit floating point).
pub type Real = f64;

/// Program counter: index into the code section.
pub type Pc = i32;

/// Pointer map for garbage collection.
///
/// Each bit indicates whether the corresponding pointer-sized slot
/// in the type's memory layout holds a traced pointer.
/// Bit `i` of byte `j` covers slot `(j * 8 + i)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerMap {
    pub bytes: Vec<u8>,
}

impl PointerMap {
    /// Count the number of set bits (pointer slots) in the map.
    pub fn count_pointers(&self) -> u32 {
        self.bytes.iter().map(|b| b.count_ones()).sum()
    }
}

/// Type descriptor: defines the size and pointer layout of a
/// heap-allocated value. Used by the GC to trace live pointers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDescriptor {
    /// Index of this descriptor in the module's type section.
    pub id: u32,
    /// Size of the described type in bytes.
    pub size: Word,
    /// Pointer map for GC traversal.
    pub pointer_map: PointerMap,
    /// Number of pointer slots in this type (derived from pointer_map).
    pub pointer_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_map_empty() {
        let pm = PointerMap { bytes: vec![] };
        assert!(pm.bytes.is_empty());
    }

    #[test]
    fn type_descriptor_basic() {
        let td = TypeDescriptor {
            id: 0,
            size: 16,
            pointer_map: PointerMap { bytes: vec![0x03] },
            pointer_count: 2,
        };
        assert_eq!(td.size, 16);
        assert_eq!(td.pointer_map.bytes[0] & 0x01, 1); // slot 0 is a pointer
        assert_eq!(td.pointer_map.bytes[0] & 0x02, 2); // slot 1 is a pointer
    }
}
