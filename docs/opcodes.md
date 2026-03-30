## Opcode Coverage Matrix

All 176 Dis VM opcodes are handled. Status levels:

- **Full**: Complete implementation matching the Dis VM specification
- **Simplified**: Working but with simplified semantics (e.g., no blocking)
- **Stub**: Handler exists but returns a no-op or error

### Summary

| Category | Full | Simplified | Stub | Total |
|---|---|---|---|---|
| Control flow | 13 | 0 | 1 | 14 |
| Data movement | 8 | 0 | 0 | 8 |
| Arithmetic | 16 | 0 | 0 | 16 |
| Bitwise | 16 | 0 | 0 | 16 |
| Comparison | 30 | 0 | 0 | 30 |
| Conversion | 20 | 0 | 0 | 20 |
| String | 8 | 0 | 0 | 8 |
| List | 16 | 0 | 0 | 16 |
| Heap allocation | 12 | 0 | 0 | 12 |
| Pointer and array | 9 | 0 | 0 | 9 |
| Module | 3 | 0 | 0 | 3 |
| Concurrency | 0 | 2 | 4 | 6 |
| Fixed-point | 8 | 0 | 3 | 11 |
| Exponentiation | 3 | 0 | 0 | 3 |
| Misc | 3 | 0 | 1 | 4 |
| **Total** | **165** | **2** | **9** | **176** |

### Concurrency Opcodes (not fully implemented)

| Opcode | Status | Notes |
|---|---|---|
| `Alt` | Stub | Returns first alternative |
| `Nbalt` | Stub | Returns 0 |
| `Spawn` | Stub | Logs warning, no-op |
| `Mspawn` | Stub | Logs warning, no-op |
| `Send` | Simplified | Stores value in channel buffer |
| `Recv` | Simplified | Reads last sent value |

### Fixed-Point Stubs

| Opcode | Status | Notes |
|---|---|---|
| `Mulx1` | Stub | Not implemented in C++ reference either |
| `Divx1` | Stub | Not implemented in C++ reference either |
| `Cvtxx1` | Stub | Not implemented in C++ reference either |
