## Mapping from Dis VM Specification to RiceVM Internals

This document maps concepts from the Dis VM specification to their implementations in RiceVM.

### Module Format

| Dis Spec Concept                    | RiceVM Location                          | Notes                                               |
|-------------------------------------|------------------------------------------|-----------------------------------------------------|
| Module header (magic, flags, sizes) | `rice-core/src/module.rs::Header`        | Parsed in `rice-loader/src/decode.rs::parse_header` |
| Instruction encoding                | `rice-core/src/instruction.rs`           | Opcodes in `rice-core/src/opcode.rs`                |
| Type descriptors                    | `rice-core/src/types.rs::TypeDescriptor` | Includes `pointer_count` derived from pointer map   |
| Export table                        | `rice-core/src/module.rs::ExportEntry`   | PC, frame type, signature hash, name                |
| Import table                        | `rice-core/src/module.rs::ImportModule`  | Grouped by external module                          |
| Exception handlers                  | `rice-core/src/module.rs::Handler`       | PC range, cases with wildcard support               |
| Data section                        | `rice-core/src/module.rs::DataItem`      | Bytes, words, bigs, reals, strings, arrays          |

### Address Modes

| Dis Address Mode            | RiceVM Enum                                    | Resolution                   |
|-----------------------------|------------------------------------------------|------------------------------|
| Offset indirect FP (mode 1) | `AddrTarget::Frame(abs_offset)`                | `fp_base + register1`        |
| Offset indirect MP (mode 0) | `AddrTarget::Mp(offset)`                       | `register1` into MP buffer   |
| Immediate (mode 2)          | `AddrTarget::Immediate`                        | Value in `imm_src`/`imm_dst` |
| None (mode 3)               | `AddrTarget::None`                             | Unused operand               |
| Double indirect FP (mode 5) | `AddrTarget::Frame` or `AddrTarget::HeapArray` | Pointer chase through FP     |
| Double indirect MP (mode 4) | `AddrTarget::Mp` or `AddrTarget::HeapArray`    | Pointer chase through MP     |

### Memory Model

| Dis Spec            | RiceVM Implementation                                    |
|---------------------|----------------------------------------------------------|
| Frame pointer (FP)  | `FrameStack::data` flat buffer + `current_data_offset()` |
| Module pointer (MP) | `VmState::mp` (Vec<u8>)                                  |
| Heap objects        | `Heap` (HashMap<HeapId, HeapObject>) with ref counting   |
| Garbage collection  | `gc.rs`: mark-and-sweep over frames, MP, loaded modules  |
| Pointer tracking    | `PointerMap` bitmask in `TypeDescriptor`                 |

### Heap Object Types

| Dis Type     | HeapData Variant                               |
|--------------|------------------------------------------------|
| Record (adt) | `Record(Vec<u8>)` or `Adt { tag, data }`       |
| String       | `Str(String)`                                  |
| Array        | `Array { elem_type, elem_size, data, length }` |
| List         | `List { head, tail }`                          |
| Channel      | `Channel { elem_size, pending }`               |
| Module ref   | `ModuleRef`, `MainModule`, `LoadedModule`      |

### Built-in Modules

| Dis Module | RiceVM File                | Function Count                    |
|------------|----------------------------|-----------------------------------|
| `$Sys`     | `rice-execute/src/sys.rs`  | 43 (all implemented)              |
| `$Math`    | `rice-execute/src/math.rs` | 66 (all implemented)              |
| `$Draw`    | `rice-execute/src/draw.rs` | 62 (SDL2 backend)                 |
| `$Tk`      | `rice-execute/src/tk.rs`   | 10 (widget tree with pack layout) |

### Concurrency

| Dis Spec          | RiceVM Implementation                                                              |
|-------------------|------------------------------------------------------------------------------------|
| `spawn` opcode    | Cooperative inline execution (default) or `PreemptiveScheduler` (with `--threads`) |
| Channels          | `HeapData::Channel` with single-slot buffer; `ChannelTable` for blocking mode      |
| `alt`/`nbalt`     | Simplified table scan over channel operations                                      |
| Thread scheduling | Round-robin with 2048-instruction quanta; OS thread pool via `std::thread::scope`  |

### Instruction Categories

| Category                            | Opcode Count | RiceVM File                                  |
|-------------------------------------|--------------|----------------------------------------------|
| Arithmetic (word, byte, big, float) | 40+          | `ops/arith.rs`, `ops/float.rs`, `ops/big.rs` |
| Comparison and branching            | 30+          | `ops/compare.rs`                             |
| Data movement                       | 10+          | `ops/data_move.rs`                           |
| String operations                   | 7            | `ops/string.rs`                              |
| List operations                     | 15           | `ops/list.rs`                                |
| Array/pointer operations            | 10+          | `ops/pointer.rs`, `ops/heap.rs`              |
| Control flow                        | 15+          | `ops/control.rs`                             |
| Type conversions                    | 20+          | `ops/convert.rs`                             |
| Fixed-point math                    | 9            | `ops/fixedpoint.rs`                          |
| Concurrency                         | 6            | `ops/concurrency.rs`                         |
