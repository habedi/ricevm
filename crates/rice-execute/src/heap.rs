//! Heap allocator with reference counting.
//!
//! Each heap object has a unique `HeapId` (u32). ID 0 is the nil sentinel.
//! Pointers in frames are stored as `Word` (i32) and cast to `HeapId` via `as u32`.

use std::collections::HashMap;

/// Handle to a heap-allocated object. 0 = nil.
pub(crate) type HeapId = u32;

/// The nil heap pointer.
pub(crate) const NIL: HeapId = 0;

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
    /// A loaded module handle.
    ModuleRef { module_id: u32 },
    /// A Dis channel (stub for milestone 3).
    Channel,
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
            next_id: 1, // 0 is reserved for NIL
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
            obj.ref_count == 0
        } else {
            false
        };
        if should_free {
            self.objects.remove(&id);
        }
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
}
