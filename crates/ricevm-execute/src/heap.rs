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
/// HeapIds start above the whole virtual address range used for frame offsets
/// and module MP addresses (`address::MP_LIMIT`), so a heap pointer can never
/// alias an MP address and vice versa. Double-indirect addressing distinguishes
/// heap pointers from frame/MP offsets by checking `value >= HEAP_ID_BASE`.
pub(crate) const HEAP_ID_BASE: HeapId = 0x1000_0000; // 256 MB

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

    /// Objects that outlive their reference count. Module handles persist for
    /// the VM lifetime because movmp/movm don't ref count embedded pointers,
    /// and they are reached through the module tables rather than memory.
    fn is_permanent(data: &HeapData) -> bool {
        matches!(
            data,
            HeapData::ModuleRef { .. }
                | HeapData::MainModule { .. }
                | HeapData::LoadedModule { .. }
        )
    }

    /// Collect the heap references owned by an object that is being freed.
    fn child_refs(&self, data: &HeapData, out: &mut Vec<HeapId>) {
        let scan = |buf: &[u8], out: &mut Vec<HeapId>| {
            let mut offset = 0;
            while offset + 4 <= buf.len() {
                let word = crate::memory::read_word(buf, offset) as HeapId;
                if word != NIL && self.objects.contains_key(&word) {
                    out.push(word);
                }
                offset += 4;
            }
        };
        match data {
            HeapData::Record(data) | HeapData::Array { data, .. } | HeapData::Adt { data, .. } => {
                scan(data, out)
            }
            HeapData::List { head, tail } => {
                scan(head, out);
                if *tail != NIL {
                    out.push(*tail);
                }
            }
            HeapData::ArraySlice { parent_id, .. } => {
                if *parent_id != NIL {
                    out.push(*parent_id);
                }
            }
            // Channel payloads are copied in by `send` without an inc_ref, so
            // there is no reference here to release.
            HeapData::Str(_)
            | HeapData::Channel { .. }
            | HeapData::ModuleRef { .. }
            | HeapData::MainModule { .. }
            | HeapData::LoadedModule { .. } => {}
        }
    }

    /// Decrement the reference count. Frees the object if it reaches 0.
    /// Freeing cascades: the references an object owns (list tails, record and
    /// array fields, a slice's parent) are released too. No-op for NIL.
    pub fn dec_ref(&mut self, id: HeapId) {
        if id == NIL {
            return;
        }
        // Iterative rather than recursive: a long list would otherwise blow the
        // native stack when its last reference goes away.
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            let should_free = if let Some(obj) = self.objects.get_mut(&id) {
                obj.ref_count = obj.ref_count.saturating_sub(1);
                obj.ref_count == 0 && !Self::is_permanent(&obj.data)
            } else {
                false
            };
            if should_free && let Some(obj) = self.objects.remove(&id) {
                self.child_refs(&obj.data, &mut pending);
            }
        }
    }

    /// Return the byte length of an array or array slice.
    pub fn array_byte_len(&self, id: HeapId) -> Option<usize> {
        let obj = self.get(id)?;
        match &obj.data {
            HeapData::Array { data, .. } => Some(data.len()),
            HeapData::ArraySlice {
                elem_size, length, ..
            } => Some(length * elem_size),
            _ => None,
        }
    }

    /// Read bytes from an array or array slice, resolving slices to their parent.
    pub fn array_read(&self, id: HeapId, offset: usize, len: usize) -> Option<Vec<u8>> {
        let obj = self.get(id)?;
        match &obj.data {
            HeapData::Array { data, .. } => {
                // `offset` can come from bytecode: a plain `offset + len` would
                // wrap and let an invalid range through the bounds check.
                match offset.checked_add(len) {
                    Some(end) if end <= data.len() => Some(data[offset..end].to_vec()),
                    _ => Some(vec![0u8; len]),
                }
            }
            HeapData::ArraySlice {
                parent_id,
                byte_start,
                ..
            } => match byte_start.checked_add(offset) {
                Some(parent_offset) => self.array_read(*parent_id, parent_offset, len),
                None => Some(vec![0u8; len]),
            },
            _ => None,
        }
    }

    /// Write bytes to an array or array slice, resolving slices to their parent.
    pub fn array_write(&mut self, id: HeapId, offset: usize, data: &[u8]) {
        if let Some(obj) = self.get(id)
            && let HeapData::ArraySlice {
                parent_id,
                byte_start,
                ..
            } = &obj.data
        {
            let pid = *parent_id;
            let bs = *byte_start;
            if let Some(parent_offset) = bs.checked_add(offset) {
                self.array_write(pid, parent_offset, data);
            }
            return;
        }
        if let Some(obj) = self.get_mut(id)
            && let HeapData::Array { data: arr_data, .. } = &mut obj.data
        {
            let Some(end) = offset.checked_add(data.len()) else {
                return;
            };
            let end = end.min(arr_data.len());
            let copy_len = end.saturating_sub(offset);
            if copy_len > 0 {
                arr_data[offset..offset + copy_len].copy_from_slice(&data[..copy_len]);
            }
        }
    }

    /// Get the mutable data buffer of an array, resolving slices to their
    /// parent. Returns (data, byte_offset) where byte_offset is the start
    /// offset within the parent's data for slice types, or 0 for arrays.
    #[allow(dead_code)]
    pub fn array_data_mut(&mut self, id: HeapId) -> Option<(&mut Vec<u8>, usize)> {
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
    /// Module handles are exempt, matching `dec_ref`: they stay alive for the
    /// VM lifetime and are not reachable from any scanned buffer.
    #[allow(dead_code)]
    pub fn sweep(&mut self, marked: &std::collections::HashSet<HeapId>) {
        self.objects
            .retain(|id, obj| marked.contains(id) || Self::is_permanent(&obj.data));
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

    #[test]
    fn alloc_string_and_retrieve() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Str("hello world".to_string()));
        assert_ne!(id, NIL);
        let obj = heap.get(id).unwrap();
        assert_eq!(obj.ref_count, 1);
        assert_eq!(obj.type_id, 0);
        match &obj.data {
            HeapData::Str(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected Str"),
        }
    }

    #[test]
    fn alloc_list_and_retrieve() {
        let mut heap = Heap::new();
        let id = heap.alloc(
            5,
            HeapData::List {
                head: vec![1, 2, 3, 4],
                tail: NIL,
            },
        );
        let obj = heap.get(id).unwrap();
        assert_eq!(obj.type_id, 5);
        match &obj.data {
            HeapData::List { head, tail } => {
                assert_eq!(head, &[1, 2, 3, 4]);
                assert_eq!(*tail, NIL);
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn alloc_channel_and_retrieve() {
        let mut heap = Heap::new();
        let id = heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );
        let obj = heap.get(id).unwrap();
        match &obj.data {
            HeapData::Channel { elem_size, pending } => {
                assert_eq!(*elem_size, 4);
                assert!(pending.is_none());
            }
            _ => panic!("expected Channel"),
        }
    }

    #[test]
    fn inc_ref_increases_count() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Record(vec![0; 8]));
        assert_eq!(heap.get(id).unwrap().ref_count, 1);
        heap.inc_ref(id);
        assert_eq!(heap.get(id).unwrap().ref_count, 2);
        heap.inc_ref(id);
        assert_eq!(heap.get(id).unwrap().ref_count, 3);
    }

    #[test]
    fn dec_ref_decreases_count() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Record(vec![0; 8]));
        heap.inc_ref(id);
        heap.inc_ref(id);
        assert_eq!(heap.get(id).unwrap().ref_count, 3);
        heap.dec_ref(id);
        assert_eq!(heap.get(id).unwrap().ref_count, 2);
        heap.dec_ref(id);
        assert_eq!(heap.get(id).unwrap().ref_count, 1);
    }

    #[test]
    fn dec_ref_to_zero_frees_object() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Record(vec![0; 8]));
        assert!(heap.contains(id));
        heap.dec_ref(id);
        assert!(!heap.contains(id));
        assert!(heap.get(id).is_none());
    }

    #[test]
    fn dec_ref_to_zero_frees_string() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Str("temp".to_string()));
        heap.dec_ref(id);
        assert!(!heap.contains(id));
    }

    #[test]
    fn dec_ref_to_zero_frees_array() {
        let mut heap = Heap::new();
        let id = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: vec![0; 16],
                length: 4,
            },
        );
        heap.dec_ref(id);
        assert!(!heap.contains(id));
    }

    #[test]
    fn contains_returns_true_for_live_objects() {
        let mut heap = Heap::new();
        let id1 = heap.alloc(0, HeapData::Record(vec![0; 4]));
        let id2 = heap.alloc(0, HeapData::Str("test".to_string()));
        assert!(heap.contains(id1));
        assert!(heap.contains(id2));
    }

    #[test]
    fn contains_returns_false_for_nil_and_freed() {
        let mut heap = Heap::new();
        assert!(!heap.contains(NIL));
        let id = heap.alloc(0, HeapData::Record(vec![0; 4]));
        heap.dec_ref(id);
        assert!(!heap.contains(id));
        // Non-existent ID
        assert!(!heap.contains(999));
    }

    #[test]
    fn get_string_returns_none_for_non_string() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Record(vec![0; 8]));
        assert_eq!(heap.get_string(id), None);
    }

    #[test]
    fn get_string_returns_none_for_nil() {
        let heap = Heap::new();
        assert_eq!(heap.get_string(NIL), None);
    }

    #[test]
    fn array_byte_len_for_array() {
        let mut heap = Heap::new();
        let id = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: vec![0; 20],
                length: 5,
            },
        );
        assert_eq!(heap.array_byte_len(id), Some(20));
    }

    #[test]
    fn array_byte_len_for_array_slice() {
        let mut heap = Heap::new();
        let parent_id = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: vec![0; 40],
                length: 10,
            },
        );
        let slice_id = heap.alloc(
            0,
            HeapData::ArraySlice {
                parent_id,
                byte_start: 8,
                elem_type: 0,
                elem_size: 4,
                length: 3,
            },
        );
        // slice byte len = length * elem_size = 3 * 4 = 12
        assert_eq!(heap.array_byte_len(slice_id), Some(12));
    }

    #[test]
    fn array_byte_len_returns_none_for_non_array() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Record(vec![0; 8]));
        assert_eq!(heap.array_byte_len(id), None);
    }

    #[test]
    fn array_byte_len_returns_none_for_nil() {
        let heap = Heap::new();
        assert_eq!(heap.array_byte_len(NIL), None);
    }

    #[test]
    fn loaded_module_not_freed_on_dec_ref() {
        let mut heap = Heap::new();
        let id = heap.alloc(
            0,
            HeapData::LoadedModule {
                module_idx: 0,
                func_map: Vec::new(),
            },
        );
        heap.dec_ref(id);
        assert!(
            heap.contains(id),
            "LoadedModule should persist at ref_count 0"
        );
    }

    #[test]
    fn main_module_not_freed_on_dec_ref() {
        let mut heap = Heap::new();
        let id = heap.alloc(
            0,
            HeapData::MainModule {
                func_map: Vec::new(),
            },
        );
        heap.dec_ref(id);
        assert!(
            heap.contains(id),
            "MainModule should persist at ref_count 0"
        );
    }

    #[test]
    fn heap_len_tracks_live_objects() {
        let mut heap = Heap::new();
        assert_eq!(heap.len(), 0);
        let id1 = heap.alloc(0, HeapData::Record(vec![0; 4]));
        assert_eq!(heap.len(), 1);
        let _id2 = heap.alloc(0, HeapData::Record(vec![0; 4]));
        assert_eq!(heap.len(), 2);
        heap.dec_ref(id1);
        assert_eq!(heap.len(), 1);
    }

    #[test]
    fn array_read_with_huge_offset_returns_zeros() {
        let mut heap = Heap::new();
        let id = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data: vec![1, 2, 3, 4],
                length: 4,
            },
        );
        // `offset + len` must not wrap past the bounds check.
        assert_eq!(heap.array_read(id, usize::MAX - 1, 4), Some(vec![0u8; 4]));
    }

    #[test]
    fn array_write_with_huge_offset_is_ignored() {
        let mut heap = Heap::new();
        let id = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data: vec![1, 2, 3, 4],
                length: 4,
            },
        );
        heap.array_write(id, usize::MAX - 1, &[9, 9, 9, 9]);
        assert_eq!(heap.array_read(id, 0, 4), Some(vec![1, 2, 3, 4]));
    }

    #[test]
    fn sweep_keeps_module_objects() {
        let mut heap = Heap::new();
        let module_ref = heap.alloc(
            0,
            HeapData::ModuleRef {
                module_id: 1,
                func_map: Vec::new(),
            },
        );
        let main_module = heap.alloc(
            0,
            HeapData::MainModule {
                func_map: Vec::new(),
            },
        );
        let loaded = heap.alloc(
            0,
            HeapData::LoadedModule {
                module_idx: 0,
                func_map: Vec::new(),
            },
        );
        let plain = heap.alloc(0, HeapData::Record(vec![0; 4]));

        heap.sweep(&std::collections::HashSet::new());

        // Module handles are exempt from dec_ref freeing, so the sweep must
        // keep them too; they are reached through the module tables, not memory.
        assert!(heap.contains(module_ref), "ModuleRef must survive sweep");
        assert!(heap.contains(main_module), "MainModule must survive sweep");
        assert!(heap.contains(loaded), "LoadedModule must survive sweep");
        assert!(!heap.contains(plain), "unmarked objects must be swept");
    }

    #[test]
    fn dec_ref_releases_list_chain() {
        let mut heap = Heap::new();
        let elem = heap.alloc(0, HeapData::Str("elem".to_string()));
        let tail = heap.alloc(
            0,
            HeapData::List {
                head: vec![0; 4],
                tail: NIL,
            },
        );
        let mut head = vec![0u8; 4];
        crate::memory::write_word(&mut head, 0, elem as i32);
        let node = heap.alloc(0, HeapData::List { head, tail });

        heap.dec_ref(node);

        assert!(!heap.contains(node));
        assert!(!heap.contains(tail), "list tail must be released with node");
        assert!(!heap.contains(elem), "list element must be released");
    }

    #[test]
    fn dec_ref_releases_record_children() {
        let mut heap = Heap::new();
        let child = heap.alloc(0, HeapData::Str("child".to_string()));
        let mut data = vec![0u8; 8];
        crate::memory::write_word(&mut data, 4, child as i32);
        let record = heap.alloc(0, HeapData::Record(data));

        heap.dec_ref(record);

        assert!(!heap.contains(record));
        assert!(!heap.contains(child), "record field must be released");
    }

    #[test]
    fn dec_ref_releases_slice_parent() {
        let mut heap = Heap::new();
        let parent = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: vec![0; 16],
                length: 4,
            },
        );
        let slice = heap.alloc(
            0,
            HeapData::ArraySlice {
                parent_id: parent,
                byte_start: 0,
                elem_type: 0,
                elem_size: 4,
                length: 2,
            },
        );
        heap.inc_ref(parent); // the slice holds a reference to its parent

        heap.dec_ref(slice);

        assert!(!heap.contains(slice));
        assert_eq!(
            heap.get(parent).map(|obj| obj.ref_count),
            Some(1),
            "freeing a slice must release its parent reference"
        );
    }

    #[test]
    fn dec_ref_releases_long_list_without_overflowing_the_stack() {
        let mut heap = Heap::new();
        let mut tail = NIL;
        for _ in 0..100_000 {
            tail = heap.alloc(
                0,
                HeapData::List {
                    head: vec![0; 4],
                    tail,
                },
            );
        }
        heap.dec_ref(tail);
        assert_eq!(heap.len(), 0, "the whole list must be released");
    }

    #[test]
    fn dec_ref_keeps_shared_children_alive() {
        let mut heap = Heap::new();
        let child = heap.alloc(0, HeapData::Str("shared".to_string()));
        heap.inc_ref(child); // held by two records
        let mut data = vec![0u8; 4];
        crate::memory::write_word(&mut data, 0, child as i32);
        let record = heap.alloc(0, HeapData::Record(data));

        heap.dec_ref(record);

        assert!(
            heap.contains(child),
            "a child with remaining references must stay alive"
        );
        assert_eq!(heap.get(child).unwrap().ref_count, 1);
    }

    #[test]
    fn multiple_allocs_get_unique_ids() {
        let mut heap = Heap::new();
        let id1 = heap.alloc(0, HeapData::Record(vec![0; 4]));
        let id2 = heap.alloc(0, HeapData::Record(vec![0; 4]));
        let id3 = heap.alloc(0, HeapData::Str("x".to_string()));
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
        assert_ne!(id1, NIL);
    }
}
