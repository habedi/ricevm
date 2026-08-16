//! Heap allocator with reference counting.
//!
//! Each heap object has a unique `HeapId` (u32). ID 0 is the nil sentinel.
//! Pointers in frames are stored as `Word` (i32) and cast to `HeapId` via `as u32`.

use std::collections::HashMap;
use std::sync::Arc;

/// Handle to a heap-allocated object. 0 = nil.
pub(crate) type HeapId = u32;

/// Identifies a type descriptor: `(module virtual index, type index)`.
///
/// A bare type index is ambiguous -- index 3 of the main module and index 3 of
/// a loaded module describe different types -- so interned maps are keyed by
/// the module that supplied them. See `VmState::current_module_virt_idx`.
pub(crate) type TypeKey = (usize, u32);

/// Which words of a heap object's buffer hold traced pointers.
///
/// Built once per type from the allocating module's `TypeDescriptor`, then
/// shared (`Arc`) by every object of that type. The bit order is the one the
/// .dis format uses: the Limbo compiler sets
/// `map[offset / 32] |= 1 << (7 - (offset / 4) % 8)` (`limbo/types.c`,
/// `tdescmap`), and the reference collector reads it back the same way
/// (`markheap` and `freeptrs` in `libinterp`). Checked against the 160 Inferno
/// modules under `external/`: of 19,825 set bits, every one names a word
/// inside its type when read most-significant-bit first, while reading it
/// least-significant-bit first puts 3,279 of them outside the type entirely.
#[derive(Debug)]
pub(crate) struct TraceMap {
    /// Byte offsets, within one element, of the words that hold pointers.
    offsets: Vec<usize>,
    /// Byte size of one element. A record has a single element; an array
    /// repeats the map every `stride` bytes.
    stride: usize,
}

impl TraceMap {
    /// Build a map from a type descriptor's pointer map and element size.
    pub fn new(map_bytes: &[u8], stride: usize) -> Self {
        let mut offsets = Vec::new();
        for (byte_idx, &map_byte) in map_bytes.iter().enumerate() {
            for bit in 0..8usize {
                if map_byte & (0x80 >> bit) == 0 {
                    continue;
                }
                let offset = (byte_idx * 8 + bit) * 4;
                // A bit past the end of the element describes nothing; it must
                // not be projected onto the following element.
                if offset + 4 <= stride {
                    offsets.push(offset);
                }
            }
        }
        Self { offsets, stride }
    }

    /// Byte offsets of the pointer words in a buffer of `buf_len` bytes.
    pub fn pointer_offsets(&self, buf_len: usize) -> impl Iterator<Item = usize> + '_ {
        // `div_ceil` rather than `/`: a buffer that does not divide evenly into
        // elements still has pointer slots in its last, partial element, and
        // every offset is bounds-checked below anyway.
        let elements = if self.stride == 0 {
            0
        } else {
            buf_len.div_ceil(self.stride)
        };
        (0..elements).flat_map(move |element| {
            let base = element * self.stride;
            self.offsets
                .iter()
                .map(move |offset| base + offset)
                .filter(move |offset| offset + 4 <= buf_len)
        })
    }
}

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
    /// Which words of this object's buffer hold pointers, resolved from the
    /// allocating module's type descriptor at allocation time -- the only
    /// moment at which both the type index and the module are known.
    /// `None` means the layout is unknown and the buffer is scanned
    /// conservatively (`gc::mark_all`).
    pub trace: Option<Arc<TraceMap>>,
    pub data: HeapData,
}

/// The VM heap: a map from HeapId to HeapObject.
pub(crate) struct Heap {
    objects: HashMap<HeapId, HeapObject>,
    next_id: HeapId,
    /// One `TraceMap` per type, shared by every object of that type.
    trace_maps: HashMap<TypeKey, Arc<TraceMap>>,
}

impl Heap {
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            next_id: HEAP_ID_BASE,
            trace_maps: HashMap::new(),
        }
    }

    /// Allocate a new heap object with no known layout. Returns its HeapId.
    pub fn alloc(&mut self, type_id: u32, data: HeapData) -> HeapId {
        self.alloc_typed(type_id, data, None)
    }

    /// Allocate a new heap object, recording which of its words are pointers.
    pub fn alloc_typed(
        &mut self,
        type_id: u32,
        data: HeapData,
        trace: Option<Arc<TraceMap>>,
    ) -> HeapId {
        let id = self.next_id;
        self.next_id += 1;
        self.objects.insert(
            id,
            HeapObject {
                ref_count: 1,
                type_id,
                trace,
                data,
            },
        );
        id
    }

    /// The interned map for a type, if one has been built already.
    pub fn trace_map(&self, key: TypeKey) -> Option<Arc<TraceMap>> {
        self.trace_maps.get(&key).cloned()
    }

    /// Intern a type's map so every object of that type can share it.
    pub fn intern_trace_map(&mut self, key: TypeKey, map: TraceMap) -> Arc<TraceMap> {
        self.trace_maps
            .entry(key)
            .or_insert_with(|| Arc::new(map))
            .clone()
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

    /// Collect the heap references *owned* by an object that is being freed.
    ///
    /// Only slots whose reference was demonstrably acquired on store belong
    /// here. Releasing anything else is a use-after-free, and the counts here
    /// are the only thing standing between the guest and a dangling id.
    ///
    /// Owned, and therefore cascaded:
    /// - `List::tail` — every cons op inc_refs the tail (ops/list.rs).
    /// - `ArraySlice::parent_id` — `slicea` inc_refs the parent (ops/pointer.rs).
    ///
    /// *Not* cascaded: the byte buffers of `List` heads, records, arrays, ADTs
    /// and channel payloads -- including the slots a `TraceMap` marks as
    /// pointers. The map is a statement about *layout*, not about ownership:
    /// it says a word may hold a pointer, not that a reference was taken when
    /// one was stored there. Nothing on the block-write paths counts:
    /// - `cons_bytes` copies the head block verbatim and inc_refs only the
    ///   tail, so `l = rec :: l` puts `rec`'s pointer fields in the head with
    ///   no reference taken on them.
    /// - `heap_write`/`array_write`/`movm` fill records and arrays with bytes
    ///   that were never ref counted -- `sys->pipe` writes two FD records
    ///   straight into the guest's `array of ref Sys->FD` this way.
    /// - the reverse direction is uncounted too: `movm` and `headm` copy a
    ///   block *out* of an object into a frame, duplicating any pointer in it,
    ///   and `op_ret` does not release a frame's pointers (ops/control.rs), so
    ///   there is no balancing release to pair a cascade with.
    ///
    /// Making this precise means ref counting every one of those paths in both
    /// directions; until then the pointers a buffer owns -- those stored by
    /// `movp`, or by `movmp`'s pointer-map walk -- are reclaimed by the
    /// mark-and-sweep pass in `gc.rs` instead, which since it traces mapped
    /// objects precisely no longer keeps them alive on a coincidence. A leak
    /// the collector can clean up is strictly safer than a reference released
    /// twice.
    fn child_refs(&self, data: &HeapData, out: &mut Vec<HeapId>) {
        match data {
            HeapData::List { tail, .. } => {
                if *tail != NIL {
                    out.push(*tail);
                }
            }
            HeapData::ArraySlice { parent_id, .. } => {
                if *parent_id != NIL {
                    out.push(*parent_id);
                }
            }
            HeapData::Record(_)
            | HeapData::Array { .. }
            | HeapData::Adt { .. }
            | HeapData::Str(_)
            | HeapData::Channel { .. }
            | HeapData::ModuleRef { .. }
            | HeapData::MainModule { .. }
            | HeapData::LoadedModule { .. } => {}
        }
    }

    /// Decrement the reference count. Frees the object if it reaches 0.
    /// Freeing cascades through the references the object owns -- a list tail,
    /// a slice's parent -- and only those; see `child_refs`. No-op for NIL.
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
        assert!(
            heap.contains(elem),
            "a cons head is copied in without a reference being taken, so \
             freeing the node must not release what it names"
        );
    }

    #[test]
    fn dec_ref_leaves_record_buffer_words_alone() {
        let mut heap = Heap::new();
        let child = heap.alloc(0, HeapData::Str("child".to_string()));
        let mut data = vec![0u8; 8];
        crate::memory::write_word(&mut data, 4, child as i32);
        let record = heap.alloc(0, HeapData::Record(data));

        heap.dec_ref(record);

        assert!(!heap.contains(record));
        assert!(
            heap.contains(child),
            "a record's buffer is untyped memory: the heap cannot tell an \
             owned pointer from a coincidence, so it releases neither"
        );
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
    fn dec_ref_does_not_release_a_cons_head_reference_it_never_took() {
        let mut heap = Heap::new();
        // A record with a `ref` field: the field holds the only reference to
        // `inner`, taken when the pointer was stored (`move_ptr_to_dst`).
        let inner = heap.alloc(0, HeapData::Str("still held by the guest".to_string()));
        let mut rec_data = vec![0u8; 4];
        crate::memory::write_word(&mut rec_data, 0, inner as i32);
        let rec = heap.alloc(0, HeapData::Record(rec_data));

        // `l = rec :: l` copies the record block into the list head byte for
        // byte; `cons_bytes` (ops/list.rs) inc_refs the tail and nothing else.
        let mut head = vec![0u8; 4];
        crate::memory::write_word(&mut head, 0, inner as i32);
        let node = heap.alloc(0, HeapData::List { head, tail: NIL });

        // Overwriting `l` releases the node.
        heap.dec_ref(node);

        assert!(!heap.contains(node), "the list node itself is released");
        assert!(
            heap.contains(inner),
            "dec_ref must never release a reference that was never acquired"
        );
        assert_eq!(
            heap.get(inner).unwrap().ref_count,
            1,
            "the record's field still owns the only reference"
        );
        assert!(heap.contains(rec));
    }

    #[test]
    fn dec_ref_does_not_release_array_bytes_that_look_like_ids() {
        let mut heap = Heap::new();
        let victim = heap.alloc(0, HeapData::Str("live object".to_string()));
        // An `array of byte` filled by `sys->read`, whose payload happens to
        // spell out a live heap id. No reference was ever taken on it.
        let mut data = vec![0u8; 8];
        crate::memory::write_word(&mut data, 0, victim as i32);
        let buf = heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data,
                length: 8,
            },
        );

        heap.dec_ref(buf);

        assert!(!heap.contains(buf));
        assert!(
            heap.contains(victim),
            "raw array bytes are data, not owned references"
        );
        assert_eq!(heap.get(victim).unwrap().ref_count, 1);
    }

    /// Tracing is precise; releasing still is not, and the two are deliberately
    /// asymmetric. A pointer map says which words *may* hold a pointer, not
    /// that a reference was taken when one was stored there: `heap_write`,
    /// `array_write`, `movm` and `cons_bytes` all copy bytes without counting
    /// anything. Cascading here on the strength of the map alone would release
    /// references that were never acquired.
    #[test]
    fn dec_ref_does_not_cascade_through_a_mapped_pointer_slot() {
        let mut heap = Heap::new();
        let child = heap.alloc(0, HeapData::Str("named by a pointer slot".to_string()));
        let mut data = vec![0u8; 4];
        crate::memory::write_word(&mut data, 0, child as i32);
        let record = heap.alloc_typed(
            0,
            HeapData::Record(data),
            Some(Arc::new(TraceMap::new(&[0x80], 4))),
        );

        heap.dec_ref(record);

        assert!(!heap.contains(record));
        assert_eq!(
            heap.get(child).map(|obj| obj.ref_count),
            Some(1),
            "a mapped slot is not proof that this object owns the reference"
        );
    }

    /// `sys->pipe` writes two FD records straight into the guest's
    /// `array of ref Sys->FD` with `array_write`, taking no reference on
    /// either. The slots are mapped -- the collector traces them -- but the
    /// array does not own them.
    #[test]
    fn dec_ref_does_not_cascade_through_a_mapped_array_element() {
        let mut heap = Heap::new();
        let fd = heap.alloc(0, HeapData::Record(vec![0; 4]));
        let mut data = vec![0u8; 8];
        crate::memory::write_word(&mut data, 0, fd as i32);
        let fds = heap.alloc_typed(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data,
                length: 2,
            },
            Some(Arc::new(TraceMap::new(&[0x80], 4))),
        );

        heap.dec_ref(fds);

        assert!(!heap.contains(fds));
        assert!(
            heap.contains(fd),
            "an uncounted store must not become a counted release"
        );
    }

    #[test]
    fn dec_ref_keeps_shared_children_alive() {
        let mut heap = Heap::new();
        let shared_tail = heap.alloc(
            0,
            HeapData::List {
                head: vec![0; 4],
                tail: NIL,
            },
        );
        // Two nodes cons onto the same tail; each cons takes a reference.
        heap.inc_ref(shared_tail);
        let first = heap.alloc(
            0,
            HeapData::List {
                head: vec![0; 4],
                tail: shared_tail,
            },
        );
        heap.inc_ref(shared_tail);
        let _second = heap.alloc(
            0,
            HeapData::List {
                head: vec![0; 4],
                tail: shared_tail,
            },
        );

        heap.dec_ref(first);

        assert!(!heap.contains(first));
        assert!(
            heap.contains(shared_tail),
            "a child with remaining references must stay alive"
        );
        assert_eq!(heap.get(shared_tail).unwrap().ref_count, 2);
    }

    /// The .dis type descriptor map is most-significant-bit first: the Limbo
    /// compiler sets `map[offset/32] |= 1 << (7 - (offset/4) % 8)`
    /// (`limbo/types.c`), and the reference collector reads it back the same
    /// way (`markheap`, `freeptrs` in `libinterp`). Reading it the other way
    /// round names slots that are not pointers, which is the difference
    /// between freeing garbage and freeing a live object.
    #[test]
    fn trace_map_reads_the_pointer_map_most_significant_bit_first() {
        let map = TraceMap::new(&[0x80], 16);
        assert_eq!(map.pointer_offsets(16).collect::<Vec<_>>(), vec![0]);

        let map = TraceMap::new(&[0x40], 16);
        assert_eq!(map.pointer_offsets(16).collect::<Vec<_>>(), vec![4]);

        let map = TraceMap::new(&[0x01], 32);
        assert_eq!(map.pointer_offsets(32).collect::<Vec<_>>(), vec![28]);

        let map = TraceMap::new(&[0x00, 0x80], 40);
        assert_eq!(map.pointer_offsets(40).collect::<Vec<_>>(), vec![32]);
    }

    #[test]
    fn trace_map_without_pointers_names_no_slots() {
        // An `array of byte` element type: size 1, no map at all.
        let map = TraceMap::new(&[], 1);
        assert!(map.pointer_offsets(64).next().is_none());
    }

    #[test]
    fn trace_map_repeats_once_per_array_element() {
        // Three elements of a type whose word 0 is a pointer and word 1 is not.
        let map = TraceMap::new(&[0x80], 8);
        assert_eq!(map.pointer_offsets(24).collect::<Vec<_>>(), vec![0, 8, 16]);
    }

    #[test]
    fn trace_map_ignores_slots_outside_the_buffer() {
        // A partial trailing element must not name a slot past the buffer.
        let map = TraceMap::new(&[0xC0], 8);
        assert_eq!(map.pointer_offsets(12).collect::<Vec<_>>(), vec![0, 4, 8]);
    }

    #[test]
    fn trace_map_ignores_map_bits_past_the_element() {
        // A bit for word 3 of a 8-byte element describes no slot of that
        // element and must not be projected onto the next one.
        let map = TraceMap::new(&[0x90], 8);
        assert_eq!(map.pointer_offsets(16).collect::<Vec<_>>(), vec![0, 8]);
    }

    #[test]
    fn trace_map_with_zero_stride_names_no_slots() {
        // A zero-sized element type would otherwise loop forever.
        let map = TraceMap::new(&[0x80], 0);
        assert!(map.pointer_offsets(64).next().is_none());
    }

    #[test]
    fn alloc_records_no_trace_map_by_default() {
        let mut heap = Heap::new();
        let id = heap.alloc(0, HeapData::Record(vec![0; 16]));
        assert!(
            heap.get(id).unwrap().trace.is_none(),
            "an allocation with no known descriptor stays conservatively scanned"
        );
    }

    #[test]
    fn alloc_typed_keeps_the_trace_map_on_the_object() {
        let mut heap = Heap::new();
        let map = std::sync::Arc::new(TraceMap::new(&[0x40], 8));
        let id = heap.alloc_typed(0, HeapData::Record(vec![0; 8]), Some(map));
        let trace = heap.get(id).unwrap().trace.as_ref().expect("map kept");
        assert_eq!(trace.pointer_offsets(8).collect::<Vec<_>>(), vec![4]);
    }

    #[test]
    fn interned_trace_maps_are_shared_between_objects() {
        let mut heap = Heap::new();
        assert!(heap.trace_map((0, 7)).is_none());
        let first = heap.intern_trace_map((0, 7), TraceMap::new(&[0x80], 4));
        let second = heap.trace_map((0, 7)).expect("cached after interning");
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "objects of one type must share a single map, not copy a Vec each"
        );
        assert!(
            heap.trace_map((1, 7)).is_none(),
            "type index 7 of another module is a different type"
        );
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
