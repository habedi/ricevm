//! Heap allocator with reference counting.
//!
//! Each heap object has a unique `HeapId` (u32). ID 0 is the nil sentinel.
//! Pointers in frames are stored as `Word` (i32) and cast to `HeapId` via `as u32`.

use std::collections::HashMap;

/// Handle to a heap-allocated object. 0 = nil.
pub(crate) type HeapId = u32;

/// The nil heap pointer.
pub(crate) const NIL: HeapId = 0;

/// Base value for HeapId allocation.
///
/// HeapIds start at this value so they never overlap with frame byte offsets
/// (typically < 1 MB) or MP offsets (typically < 100 KB). This allows
/// double-indirect addressing to distinguish heap pointers from frame/MP
/// offsets by checking `value >= HEAP_ID_BASE`.
pub(crate) const HEAP_ID_BASE: HeapId = 0x0100_0000; // 16 MB

/// The kind of data stored in a heap object.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum HeapData {
    /// A typed record (from `new`/`newz`), sized by TypeDescriptor.
    Record(Vec<u8>),
    /// A Dis string.
    Str(String),
    /// A Dis array.
    Array {
        elem_type: u32,
        elem_size: usize,
        data: Vec<u8>,
        length: usize,
    },
    /// A Dis list node (head + tail).
    /// `head` is a raw byte buffer holding the element value.
    /// `tail` is the HeapId of the next list node (NIL = end of list).
    List { head: Vec<u8>, tail: HeapId },
    /// A loaded module handle (for built-in modules).
    /// `func_map` maps import function indices to builtin function indices.
    ModuleRef {
        module_id: u32,
        func_map: Vec<Option<usize>>,
    },
    /// A reference to the main module being executed by the VM.
    /// `func_map` maps caller import indices to export indices.
    MainModule { func_map: Vec<Option<usize>> },
    /// A loaded Dis module from a .dis file.
    /// `func_map` maps caller's import function indices to loaded module's export indices.
    LoadedModule {
        module_idx: usize,
        func_map: Vec<Option<usize>>,
    },
    /// A Dis channel with a single pending payload.
    Channel {
        elem_size: usize,
        pending: Option<Vec<u8>>,
    },
    /// A Dis ADT (abstract data type) with a pick tag.
    /// `tag` identifies which pick variant is active (0 = base fields only).
    /// `data` contains the fields as a flat byte buffer, same layout as Record.
    Adt { tag: u32, data: Vec<u8> },
    /// A slice view into another array. Reads and writes go through to the
    /// parent array at `byte_start`. This preserves shared-storage semantics
    /// required by the Dis VM (e.g. Bufio fills a buffer slice, and getb
    /// reads from the original buffer).
    ArraySlice {
        parent_id: HeapId,
        byte_start: usize,
        elem_type: u32,
        elem_size: usize,
        length: usize,
    },
}

/// A heap-allocated object with reference count.
#[derive(Debug)]
pub(crate) struct HeapObject {
    pub ref_count: u32,
    pub type_id: u32,
    pub data: HeapData,
}

/// The VM heap: a map from HeapId to HeapObject.
pub(crate) struct Heap {
    objects: HashMap<HeapId, HeapObject>,
    next_id: HeapId,
}

impl Heap {
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            next_id: HEAP_ID_BASE,
        }
    }

    /// Allocate a new heap object. Returns its HeapId.
    pub fn alloc(&mut self, type_id: u32, data: HeapData) -> HeapId {
        let id = self.next_id;
        self.next_id += 1;
        self.objects.insert(
            id,
            HeapObject {
                ref_count: 1,
                type_id,
                data,
            },
        );
        id
    }

    /// Get a reference to a heap object. Returns None for NIL or freed objects.
    pub fn get(&self, id: HeapId) -> Option<&HeapObject> {
        if id == NIL {
            return None;
        }
        self.objects.get(&id)
    }

    /// Get a mutable reference to a heap object.
    #[allow(dead_code)]
    pub fn get_mut(&mut self, id: HeapId) -> Option<&mut HeapObject> {
        if id == NIL {
            return None;
        }
        self.objects.get_mut(&id)
    }

    /// Increment the reference count. No-op for NIL.
    pub fn inc_ref(&mut self, id: HeapId) {
        if id == NIL {
            return;
        }
        if let Some(obj) = self.objects.get_mut(&id) {
            obj.ref_count += 1;
        }
    }

    /// Decrement the reference count. Frees the object if it reaches 0.
    /// No-op for NIL.
    pub fn dec_ref(&mut self, id: HeapId) {
        if id == NIL {
            return;
        }
        let should_free = if let Some(obj) = self.objects.get_mut(&id) {
            obj.ref_count = obj.ref_count.saturating_sub(1);
            if obj.ref_count == 0 {
                // Don't free module references — they persist for the VM lifetime
                // and movmp/movm don't do proper ref counting for embedded pointers.
                !matches!(
                    obj.data,
                    HeapData::ModuleRef { .. }
                        | HeapData::MainModule { .. }
                        | HeapData::LoadedModule { .. }
                )
            } else {
                false
            }
        } else {
            false
        };
        if should_free {
            self.objects.remove(&id);
        }
    }

    /// Read bytes from an array or array slice, resolving slices to their parent.
    pub fn array_read(&self, id: HeapId, offset: usize, len: usize) -> Option<Vec<u8>> {
        let obj = self.get(id)?;
        match &obj.data {
            HeapData::Array { data, .. } => {
                if offset + len <= data.len() {
                    Some(data[offset..offset + len].to_vec())
                } else {
                    Some(vec![0u8; len])
                }
            }
            HeapData::ArraySlice {
                parent_id,
                byte_start,
                ..
            } => self.array_read(*parent_id, byte_start + offset, len),
            _ => None,
        }
    }

    /// Write bytes to an array or array slice, resolving slices to their parent.
    pub fn array_write(&mut self, id: HeapId, offset: usize, data: &[u8]) {
        if let Some(obj) = self.get(id) {
            if let HeapData::ArraySlice {
                parent_id,
                byte_start,
                ..
            } = &obj.data
            {
                let pid = *parent_id;
                let bs = *byte_start;
                self.array_write(pid, bs + offset, data);
                return;
            }
        }
        if let Some(obj) = self.get_mut(id) {
            match &mut obj.data {
                HeapData::Array {
                    data: arr_data, ..
                } => {
                    let end = (offset + data.len()).min(arr_data.len());
                    let copy_len = end.saturating_sub(offset);
                    if copy_len > 0 {
                        arr_data[offset..offset + copy_len]
                            .copy_from_slice(&data[..copy_len]);
                    }
                }
                _ => {}
            }
        }
    }

    /// Get the mutable data buffer of an array, resolving slices to their
    /// parent. Returns (data, byte_offset) where byte_offset is the start
    /// offset within the parent's data for slice types, or 0 for arrays.
    pub fn array_data_mut(
        &mut self,
        id: HeapId,
    ) -> Option<(&mut Vec<u8>, usize)> {
        // First resolve ArraySlice to its parent.
        let (root_id, byte_start) = {
            let obj = self.get(id)?;
            match &obj.data {
                HeapData::ArraySlice {
                    parent_id,
                    byte_start,
                    ..
                } => (*parent_id, *byte_start),
                HeapData::Array { .. } => (id, 0),
                _ => return None,
            }
        };
        let obj = self.get_mut(root_id)?;
        match &mut obj.data {
            HeapData::Array { data, .. } => Some((data, byte_start)),
            _ => None,
        }
    }

    /// Number of live objects on the heap.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Check if a HeapId refers to a live object.
    #[allow(dead_code)]
    pub fn contains(&self, id: HeapId) -> bool {
        id != NIL && self.objects.contains_key(&id)
    }

    /// Remove all objects not in the marked set (sweep phase of GC).
    #[allow(dead_code)]
    pub fn sweep(&mut self, marked: &std::collections::HashSet<HeapId>) {
        self.objects.retain(|id, _| marked.contains(id));
    }

    /// Get the string data from a heap object, or None if not a string.
    pub fn get_string(&self, id: HeapId) -> Option<&str> {
        match self.get(id)? {
            HeapObject {
                data: HeapData::Str(s),
                ..
            } => Some(s.as_str()),
            _ => None,
        }
    }

    /// Get mutable string data, cloning if shared (copy-on-write).
    /// Returns the (possibly new) HeapId and a mutable reference to the String.
    pub fn cow_string(&mut self, id: HeapId) -> Option<(HeapId, &mut String)> {
        if id == NIL {
            return None;
        }
        let obj = self.objects.get(&id)?;
        if !matches!(obj.data, HeapData::Str(_)) {
            return None;
        }
        if obj.ref_count > 1 {
            // Clone the string into a new object
            let cloned = match &obj.data {
                HeapData::Str(s) => s.clone(),
                _ => unreachable!(),
            };
            let type_id = obj.type_id;
            self.dec_ref(id);
            let new_id = self.alloc(type_id, HeapData::Str(cloned));
            let new_obj = self.objects.get_mut(&new_id)?;
            match &mut new_obj.data {
                HeapData::Str(s) => Some((new_id, s)),
                _ => unreachable!(),
            }
        } else {
            let obj = self.objects.get_mut(&id)?;
            match &mut obj.data {
                HeapData::Str(s) => Some((id, s)),
                _ => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_and_get() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Record(vec![0; 16]));
        assert_ne!(id, NIL);
        let obj = heap.get(id).unwrap();
        assert_eq!(obj.ref_count, 1);
        assert!(matches!(obj.data, HeapData::Record(_)));
    }

    #[test]
    fn ref_counting() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Str("hello".to_string()));
        heap.inc_ref(id);
        assert_eq!(heap.get(id).unwrap().ref_count, 2);
        heap.dec_ref(id);
        assert_eq!(heap.get(id).unwrap().ref_count, 1);
        heap.dec_ref(id);
        assert!(heap.get(id).is_none()); // freed
    }

    #[test]
    fn nil_is_safe() {
        let mut heap = Heap::new();
        assert!(heap.get(NIL).is_none());
        heap.inc_ref(NIL); // no-op
        heap.dec_ref(NIL); // no-op
    }

    #[test]
    fn get_string() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Str("test".to_string()));
        assert_eq!(heap.get_string(id), Some("test"));
    }

    #[test]
    fn cow_string_unique() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Str("hello".to_string()));
        let (new_id, s) = heap.cow_string(id).unwrap();
        assert_eq!(new_id, id); // no copy needed
        s.push_str(" world");
        assert_eq!(heap.get_string(id), Some("hello world"));
    }

    #[test]
    fn cow_string_shared() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Str("hello".to_string()));
        heap.inc_ref(id); // ref_count = 2
        let (new_id, s) = heap.cow_string(id).unwrap();
        assert_ne!(new_id, id); // copy was made
        s.push_str(" world");
        assert_eq!(heap.get_string(new_id), Some("hello world"));
        assert_eq!(heap.get_string(id), Some("hello")); // original unchanged
    }

    #[test]
    fn alloc_array() {
        let mut heap = Heap::new();
        let id = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: vec![0; 40],
                length: 10,
            },
        );
        let obj = heap.get(id).unwrap();
        match &obj.data {
            HeapData::Array { length, .. } => assert_eq!(*length, 10),
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn array_read_through_slice() {
        let mut heap = Heap::new();
        // Parent array with data [10, 20, 30, 40, 50] as i32
        let mut data = vec![0u8; 20];
        for i in 0..5i32 {
            crate::memory::write_word(&mut data, i as usize * 4, (i + 1) * 10);
        }
        let parent_id = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data,
                length: 5,
            },
        );

        // Slice starting at byte 8 (element 2), length 2
        let slice_id = heap.alloc(
            0,
            HeapData::ArraySlice {
                parent_id,
                byte_start: 8,
                elem_type: 0,
                elem_size: 4,
                length: 2,
            },
        );

        // Read first element of the slice (should be parent element 2 = 30)
        let bytes = heap.array_read(slice_id, 0, 4).unwrap();
        let val = crate::memory::read_word(&bytes, 0);
        assert_eq!(val, 30);

        // Read second element of slice (should be parent element 3 = 40)
        let bytes = heap.array_read(slice_id, 4, 4).unwrap();
        let val = crate::memory::read_word(&bytes, 0);
        assert_eq!(val, 40);
    }

    #[test]
    fn array_write_through_slice_updates_parent() {
        let mut heap = Heap::new();
        let data = vec![0u8; 20];
        let parent_id = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data,
                length: 5,
            },
        );

        // Slice at byte_start=4 (element 1)
        let slice_id = heap.alloc(
            0,
            HeapData::ArraySlice {
                parent_id,
                byte_start: 4,
                elem_type: 0,
                elem_size: 4,
                length: 3,
            },
        );

        // Write 99 at slice offset 0 (= parent offset 4)
        heap.array_write(slice_id, 0, &99i32.to_le_bytes());

        // Read from parent at offset 4 -- should see 99
        let bytes = heap.array_read(parent_id, 4, 4).unwrap();
        let val = i32::from_le_bytes(bytes.try_into().unwrap());
        assert_eq!(val, 99);
    }

    #[test]
    fn module_ref_not_freed_on_dec_ref() {
        let mut heap = Heap::new();
        let id = heap.alloc(
            0,
            HeapData::ModuleRef {
                module_id: 1,
                func_map: Vec::new(),
            },
        );
        assert!(heap.contains(id));

        // Dec ref to 0 -- ModuleRef should NOT be freed
        heap.dec_ref(id);
        assert!(
            heap.contains(id),
            "ModuleRef should persist even at ref_count 0"
        );
        assert_eq!(heap.get(id).unwrap().ref_count, 0);
    }
}
