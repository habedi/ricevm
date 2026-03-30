## Architecture Overview

### Crate Structure

```
ricevm-cli (binary)
├── ricevm-core (shared types)
├── ricevm-loader → ricevm-core
└── ricevm-execute → ricevm-core, ricevm-loader
```

**ricevm-core** defines the data types that flow between the loader and executor:
`Module`, `Instruction`, `Opcode`, `TypeDescriptor`, `DataItem`, `Header`, and error types.
It contains no runtime logic.

**ricevm-loader** parses `.dis` binary files into `Module` structs. One public function:
`load(&[u8]) -> Result<Module, LoadError>`.

**ricevm-execute** runs a `Module` from its entry point. One public function:
`execute(&Module) -> Result<(), ExecError>`. Internally contains:

| Module | Purpose |
|---|---|
| `vm.rs` | `VmState` struct, execution loop, operand read/write helpers |
| `frame.rs` | `FrameStack` with two-phase push (frame/call semantics) |
| `heap.rs` | `Heap` with reference counting, copy-on-write strings |
| `address.rs` | Operand resolution: `Operand` → `AddrTarget` (frame/mp/immediate/heap) |
| `memory.rs` | Typed read/write on byte buffers |
| `data.rs` | Module data (MP) initialization from `DataItem` entries |
| `ops/` | 176 instruction handlers organized by category |
| `sys.rs` | Built-in `$Sys` module (43 functions) |
| `math.rs` | Built-in `$Math` module (66 functions) |
| `builtin.rs` | `ModuleRegistry` for built-in module registration |
| `gc.rs` | Mark-and-sweep garbage collector |
| `scheduler.rs` | Cooperative thread scheduler (infrastructure) |
| `channel.rs` | Channel data structure for inter-thread communication |

**ricevm-cli** provides the `run` and `dis` subcommands.

### Memory Model

**Frames** are flat byte buffers stored contiguously in `FrameStack.data`. Each frame has a
16-byte header (prev_pc, prev_base, reserved) followed by the data area. Operand offsets from
instructions are byte offsets into the data area.

**Module data (MP)** is a flat `Vec<u8>` initialized from `DataItem` entries at module load time.
Strings in the data section are allocated on the heap; their `HeapId` is stored in MP.

**Heap** is a `HashMap<u32, HeapObject>` with monotonically increasing IDs. HeapId 0 = nil.
Pointers in frames/MP are stored as `Word` (i32) and cast to `HeapId` via `as u32`.

### Address Resolution

Each instruction has up to 3 operands (source, middle, destination). Before dispatch,
`resolve_operands` converts each `Operand` into an `AddrTarget`:

| Mode | Target |
|---|---|
| `OffsetIndirectFp` | `Frame(fp_base + offset)` |
| `OffsetIndirectMp` | `Mp(offset)` |
| `Immediate` | `Immediate` (value in scratch slot) |
| `DoubleIndirectFp` | Dereference `frame[reg1]`, add `reg2`. May resolve to `HeapArray` |
| `DoubleIndirectMp` | Similar via MP |

The `HeapArray { id, offset }` variant enables array element access through `indx`.

### Instruction Dispatch

A single `match` on `Opcode` (176 arms) in `ops/mod.rs`. The compiler generates a jump table
for the dense `#[repr(u8)]` enum.

### Built-in Modules

Built-in modules (`$Sys`, `$Math`) register via `ModuleRegistry`. Each function has a name,
frame size, and a `fn(&mut VmState) -> Result<(), ExecError>` handler. The `load` opcode
checks built-ins first, then searches the filesystem for `.dis` files.

### Garbage Collection

Two-tier strategy:
1. **Reference counting** (always on): `movp` increments the new value and decrements the old.
2. **Mark-and-sweep** (in `gc.rs`): scans frame stack and MP for reachable HeapIds, sweeps unmarked objects.
