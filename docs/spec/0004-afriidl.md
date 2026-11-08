# AFR-0004 — AfriIDL: interface definition language and wire encoding

| | |
|---|---|
| **Status** | **v0 Implemented** (host compiler, scalar messages, Rust output); **v1 Designed** (ordinals, variable-length types, handles, message header) |
| **Version** | 0.1 (describes code at `485b863`, 2026-11-08) |
| **Editor** | Chief architect |
| **Code** | `tools/afriidl/src/main.rs`, `idl/ipc.afriidl`, `generated/ipc.rs`, `scripts/gen-idl.sh` |
| **Depends on** | AFR-0001 v1 (channels carry bytes + handles) |
| **Licence** | Apache-2.0 |

RFC 2119 key words apply.

---

## 1. Motivation

In a system where everything is IPC, the interfaces *are* the architecture. docs/04 commits to declaring
every interface once, in AfriIDL, and generating code for every language, so that services and clients
share a typed, versioned contract (the Fuchsia FIDL lesson). v0 proves the tool chain on one scalar
message. v1 must carry real protocols: block and network drivers (AFR-0002), the Tonga surface protocol,
AfriCore services (AFR-0006, AFR-0007). This document fixes the language, the wire encoding and the
evolution rules before more than one protocol depends on them.

## 2. Status summary

| Element | v0 (built) | v1 (designed) |
|---|---|---|
| Compiler | `afriidl <in.afriidl> -o <out.rs>`, host tool, Rust only | Same tool; Rust + C header + a WASM-guest binding for Nia apps |
| Files | one `protocol` per file | `library` with several protocols, `using` imports |
| Types | `u8 u16 u32 u64 i8 i16 i32 i64 bool` | + `string`, `vector<T>`, `array<T, N>`, `handle<kind>`, structs, enums, bits |
| Method ids | position in the file (0, 1, …) | explicit ordinals `@n`, mandatory |
| Message framing | none (struct bytes only) | 16-byte header (§5.1) |
| Used by the kernel | **no** — `generated/ipc.rs` is checked by the smoke suite but not compiled in | every v1 channel protocol |

## 3. v0 language (as built)

### 3.1 Grammar (EBNF)

```
file      = protocol ;
protocol  = "protocol" IDENT "{" { message | method } "}" ;
message   = "message" IDENT "{" { field [ "," ] } "}" ;
field     = IDENT ":" TYPE ;
method    = "method" IDENT "(" { field [ "," ] } ")" [ "->" TYPE ] ";" ;
TYPE      = IDENT ;            (* v0 accepts only the nine scalar names in message fields *)
IDENT     = (ALPHA | "_") { ALNUM | "_" } ;
comments  = "//" … end of line  |  "/*" … "*/" ;
```

Method parameter and return types are recorded in documentation comments only; v0 does not check them.

### 3.2 Encoding (v0)

A message is its fields **packed in declaration order, little-endian, with no padding and no header**.
`bool` is one byte: `0` false, any other value decodes as true (v1 tightens this). Size = sum of field
sizes. Generated Rust: `#[repr(C)] struct` with `WIRE_SIZE`, `to_bytes() -> [u8; WIRE_SIZE]`,
`from_bytes(&[u8]) -> Self` (asserts length).

Note: the generated struct is `#[repr(C)]`, so its *in-memory* layout may contain padding; only
`to_bytes`/`from_bytes` define the wire form. Code MUST NOT transmute a message struct to bytes.

### 3.3 Method ids (v0)

`enum <Protocol>Method : u32` with values in declaration order. **Reordering methods changes the ABI** —
the reason v1 requires explicit ordinals.

## 4. v1 language (Designed)

```
afriidl 1;
library asili.block;                       // dotted name; one library per directory
using afri.core;                           // import another library

/// Doc comments become generated docs.
protocol BlockDevice @0x0101 {             // protocol ordinal: u32, unique system-wide
    info @1 () -> BlockInfo;
    read @2 (lba: u64, count: u32, buf: handle<vmo>, off: u64) -> Status;
    write @3 (lba: u64, count: u32, buf: handle<vmo>, off: u64, fua: bool) -> Status;
    flush @4 () -> Status;
    -> media_changed @16 (present: bool);  // event (server → client), no reply
}

struct BlockInfo { block_size: u32; block_count: u64; flags: BlockFlags; }
bits BlockFlags : u32 { READ_ONLY = 0x1; REMOVABLE = 0x2; BOOT_MEDIUM = 0x4; }
enum Status : i32 { OK = 0; IO_ERROR = -1; OUT_OF_RANGE = -2; READ_ONLY = -3; }
```

Types: scalars (as v0, plus `f32 f64`); `bool` (MUST be 0 or 1 on the wire); `string` (UTF-8, optional
`:N` max length); `vector<T>:N`; `array<T, N>` (fixed, inline); `handle<kind>` with kind ∈ {`channel`,
`vmo`, `irq`, `mmio`, `bti`, `process`, `any`}; `struct`; `enum : <int>`; `bits : <unsigned>`; `T?` only
for `string`, `vector` and `handle` (nullable).

Rules: ordinals are mandatory, unique within a protocol, 1–0xFFFF_FFFF; method ordinals ≥ 0x8000_0000 are
reserved for the system (introspection, version query). A method without `-> T` is one-way.

## 5. v1 wire encoding (Designed)

### 5.1 Message header (16 bytes)

```
 0  4  txid      u32  0 for one-way calls and events; caller-chosen, non-zero, for calls; echoed in reply
 4  2  flags     u16  bit0 REPLY, bit1 EVENT, bit2 ERROR (body is a single i32 Status), others 0
 6  1  magic     u8   = 0xA1
 7  1  version   u8   = 1
 8  8  ordinal   u64  = (protocol_ordinal << 32) | method_ordinal
```

### 5.2 Body

1. **Fixed part**: fields in declaration order, each aligned to its natural alignment (1, 2, 4 or 8),
   padding bytes MUST be zero; the fixed part is padded to a multiple of 8.
2. **Out-of-line part**: for each `string`/`vector` in depth-first declaration order, its bytes, each
   object padded with zeros to a multiple of 8.
3. A `string` or `vector` occupies 8 bytes in the fixed part: `u32 offset` (from the start of the body to
   its out-of-line data) and `u32 count` (bytes for string, elements for vector). Absent (nullable) =
   offset `0xFFFF_FFFF`, count 0.
4. A `handle` occupies 4 bytes: index into the message's handle array; absent = `0xFFFF_FFFF`. Every handle
   in the array MUST be referenced exactly once.
5. Nested structs are inline. Maximum depth 32. Maximum message size and handle count are the channel
   limits (AFR-0001 §7.5: 65 536 bytes, 64 handles).

### 5.3 Decoding rules (MUST reject)

Non-zero padding; `bool` not 0/1; enum value not declared (unless the enum is marked `flexible`); `bits`
with undeclared bits set (unless `flexible`); offsets pointing outside the body or not 8-aligned; overlapping
out-of-line objects; string not valid UTF-8; string or vector longer than its declared maximum; unused or
doubly-used handles; header `magic`/`version` mismatch; unknown ordinal → reply with `ERROR` flag and
Status `-0x7FFF_FFFF` (`UNKNOWN_METHOD`).

## 6. Versioning and evolution

- `afriidl 1;` selects the language version; a file without it is v0.
- **Compatible changes**: add a method (new ordinal); add an event; add a member to a `flexible` enum/bits;
  rename anything (names are not on the wire).
- **Incompatible changes** (require a new protocol ordinal): change or remove a parameter; reorder struct
  fields; change a type; reuse an ordinal.
- Every protocol answers system method `@0x8000_0001 version() -> u32` with the highest library minor
  version it implements.
- The compiler MUST support `--check-compat old.afriidl new.afriidl` and fail on incompatible changes
  (to be built with v1).

## 7. Security considerations

- Decoders run on untrusted input (any peer). Generated decoders MUST be bounds-checked, MUST NOT allocate
  more than the message size, and MUST validate before handing a value to user code (§5.3).
- Handles move with the message (AFR-0001 §7.4); a decoder that rejects a message MUST close every handle
  it carried, so a malformed message cannot leak capabilities into the receiver's table.
- `txid` is not authentication; replies are matched per channel, and a channel is point-to-point.

## 8. Test vectors

### 8.1 v0

`Message { endpoint: 1, tag: 2, payload: 0x1122334455667788 }` (protocol `Ipc`, `idl/ipc.afriidl`):

```
01 00 00 00  02 00 00 00  88 77 66 55 44 33 22 11        (WIRE_SIZE 16)
IpcMethod::Send = 0, IpcMethod::Recv = 1
```

### 8.2 v1

Library `afri.test`, `protocol Greeter @0x0001 { hello @1 (name: string:32); }`, call with txid 7 and
name `"Hi"`:

```
header  07 00 00 00  00 00  A1  01  01 00 00 00 01 00 00 00
fixed   08 00 00 00  02 00 00 00                              offset 8, count 2
ool     48 69 00 00 00 00 00 00                               "Hi" + 6 bytes zero padding
total 32 bytes, 0 handles
```

`BlockInfo { block_size: 512, block_count: 0x800000, flags: BOOT_MEDIUM }`, v1 natural alignment:

```
00 02 00 00  00 00 00 00   00 00 80 00 00 00 00 00   04 00 00 00 00 00 00 00
block_size @0, 4 bytes padding, block_count @8, flags @16, padded to 24
```

(The same vector appears as A1-1 in AFR-0002.)

Reject vectors: the Greeter message with byte 22 (padding) = `01`; with count = 33 (over `:32`); with
offset = 4 (not 8-aligned); with header byte 6 = `A0`.

## 9. References

- Fuchsia FIDL language and wire-format specifications (heritage); Cap'n Proto encoding (out-of-line
  pointers); docs/01 §IPC, docs/04; AFR-0001, AFR-0002.
