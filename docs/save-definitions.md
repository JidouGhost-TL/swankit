# Save definitions, version 1

Each file in `crates/ws-saves/data/` describes one game's save. The engine has
no game-specific Rust modules. A definition contains layout data and a bounded
semantic program, so checksums, conditional records, redundant copies, text
codes and derived values can be represented without native game code.

The version identifier is `"schema": "swankit-save-def/1"`.
[save-definition.schema.json](save-definition.schema.json) is its JSON Schema,
using [Draft 2020-12](https://json-schema.org/draft/2020-12/json-schema-core).
Its `$id` is `urn:swankit:save-def:1`. Resolve its `$ref` values within that file;
validation needs no network. A new incompatible grammar requires a new version.

## Top-level fields

| Field | Meaning |
| --- | --- |
| `schema` | Exact version identifier above. |
| `id` | Stable title identity; use the ID in the data, rather than guessing from a filename. |
| `implementation` | `generated`; all game behavior is represented in this file. |
| `media` | Array of physical media and image lengths, in bytes. Each item has `kind` (`cartridge-sram`, `cartridge-eeprom`, `internal-eeprom`) and `sizes`, a nonempty list of supported full dump lengths. Companion EEPROM is optional unless the program requests it. |
| `fields` | Fixed stored fields, in source order. Index in this array is the edit identifier. |
| `records` | Packed record types and member layouts from the C declarations. |
| `blocks` | Groups identified by a fixed field index; `redundancy: "program"` delegates selection, majority, generation counters and replication to the semantic program. It does **not** imply that every copy is valid or interchangeable. |
| `checksums` | Named bindings to `ws-save` algorithms, with explicit parameters. See below. Composite checks, signatures, linked records and conditional checksum extents are also described in `program`. |
| `primitives` | Named bindings to parameterized engine transforms. |
| `validity` | `policy: "program"`; valid flags in the decoded tree follow the program's signature, range and checksum decisions. `exclude` names working regions that are excluded when deciding whether any stored game record is usable. Preserve their own flags in output. |
| `entry` | Decode/repair calling convention. |
| `program` | A `Module` node with ordered initialization statements and semantic functions. |

Offsets and sizes are byte coordinates, even for word-addressed EEPROM.
The EEPROM word at address `a` begins at byte `2*a`. Media geometry follows
[WSdev EEPROM](https://ws.nesdev.org/wiki/EEPROM) and
[WSdev Internal EEPROM](https://ws.nesdev.org/wiki/Internal_EEPROM).

### Fixed fields and records

Every fixed field has `name`, `medium`, `offset`, `size`, `type`, `encoding`,
`meaning`, `confidence` and `evidence`. `medium` retains the research label;
case-insensitive labels containing SRAM refer to cartridge SRAM, labels
containing internal/console/IEEP to internal EEPROM, and other EEPROM labels
to cartridge EEPROM. Select the appropriate image before reading or editing.

`type: "unsigned"` with `encoding` `u8`, `u16le` or `u32le` and matching size
is an unsigned scalar. All other fixed fields have `type: "bytes"`: the raw
adapter reads/writes an array of byte integers. Their `encoding` still describes
the semantic representation (signed values, strings, bitfields, packed records,
transformed words or other research notation). Decode those meanings using the
program; never infer that a raw edit to an encoded word is a semantic edit.
`meaning`, `confidence` and `evidence` explain research provenance, without
changing decoding. Confidence labels are source annotations, not probabilities.

Optional `array` has `count` and `stride` in bytes: element `i` starts at
`offset + i*stride`, for `0 <= i < count`. Verify `count*stride == size`.
Names are not necessarily unique: repeated rows and aliasing are preserved.
Editing by array index avoids ambiguity and does not update aliases or copies.

A record has `name`, `width` and `fields`. Each member has `name`, relative
`offset`, total `size`, C `type`, `count` and element `stride`. Integer C types
use their indicated signedness and little-endian storage; `char` is a byte.
Other types reference a preceding named packed record. A zero-count C tail
member has size zero and describes no resident elements. For record number `i`
starting at `start`, bound `[start + i*width, start + (i+1)*width)` before reading
its members. Only statically resolved packed declarations appear here.
Conditional/symbolic fields and structures are described by `program`, so the
fixed-field list is an edit index rather than the complete decoded value tree.

### Checksum bindings

A binding names `library: "ws-save"`, an `algorithm`, and `parameters`.

* `crc16`: reflected CRC. Parameters are 16-bit `polynomial`, `seed`,
  `xor_out`, boolean `swap_bytes`, and `seed_argument`. For each byte, XOR it
  into the accumulator; repeat eight times: shift right one and XOR the
  polynomial if the old low bit was one. XOR the result with `xor_out`, then
  swap its two bytes if requested. `seed_argument` identifies an optional
  positional seed override. KERMIT uses polynomial 33800, seed 0, xor-out 0;
  X-25 uses that polynomial, seed 65535, xor-out 65535.
* `fold`: `unit` is `byte` or `word-le`; `fold` is `sum` or `xor`;
  `bits` is 1..64; `seed`, `xor_out` and `offset_base` are unsigned integers.
  Start with `seed & mask`, where `mask = 2**bits - 1`. For each unit,
  add or XOR it and mask again. Finish with `(accumulator ^ xor_out) & mask`.
  For these two folds, `offset_base` does not affect the answer.

Optional `input` identifies a generator override: `argument` is the input
function's zero-based argument index, `offset` and `size` select its byte
region. The resulting `definition_checksum` call is also explicit in the
program. Without `input`, the call's second operand supplies a byte sequence
or unpacked word list. Validate word extents and ranges before computing.
CRC-bound functions dispatch directly to the library; fold-bound functions
retain the program's operand selection and call the library from their bodies.
Other checksum expressions remain portable arithmetic nodes; callers must
execute those expressions too, rather than assuming this map lists every
stored check word. The library additionally provides rotated/weighted sums,
negation, lane sum/XOR, rolling pairs and word permutations for composition.

### Transform bindings

`xor-rotate-feedback` has `bits` (2..64), `xor_mask`, `feedback_bit`,
`max_rounds` (0..65536), and `direction` (`encode` or `decode`). Require
`0 <= feedback_bit < bits-1`, and value/mask within the configured word width.
The call supplies a value and a round count bounded by `max_rounds`.

For encoding, XOR the word with `xor_mask`. At each round, remember whether
the feedback bit is zero, rotate left one within `bits`, and XOR bit zero
if the remembered bit was zero. For decoding, rotate right one; if the
feedback bit is zero, XOR the top bit. After all rounds XOR `xor_mask`.
The top bit cannot be the feedback bit because that would destroy invertibility.
All constants and the round count come from the definition or its program.

## Read and write with Rust

```rust,no_run
use serde_json::json;
use ws_saves::{DecodeContext, Format};

fn edit(definition_json: &str, image: &mut [u8], field_index: usize)
    -> formatkit_core::Result<serde_json::Value>
{
    let format = Format::from_json(definition_json)?;
    let context = DecodeContext::default();
    let before = format.decode_with_context(image, "save.eeprom", &context)?;
    let mut candidate = image.to_vec();
    format.set_field(&mut candidate, field_index, &json!(1))?;
    format.fix_with_context(&mut candidate, "save.eeprom", &context)?;
    let checks = format.verify_with_context(&candidate, "save.eeprom", &context)?;
    let after = format.decode_with_context(&candidate, "save.eeprom", &context)?;
    // Apply the definition's record-selection policy to checks and review after.
    // Commit the candidate only when the edited record is valid.
    let _ = (before, checks);
    image.copy_from_slice(&candidate);
    Ok(after)
}
```

1. Load the JSON with `Format::from_json`, or select a bundled definition by
   `ws_saves::format(id)`. `definition()` exposes the complete JSON.
2. Read the full image at the application boundary. Supply the original name
   if the program uses filename-keyed output or a sibling EEPROM name.
3. Populate `DecodeContext.tables` with required external lookup tables;
   populate `images` with companion images, keyed by the names requested by
   the program. No file access occurs inside the library.
4. Decode. An invalid image can produce a tree with `valid: false` and an
   error message, as the original grammar specifies. Execution errors return
   `Err`. A successful decode alone does not establish validity.
5. Select the stored field's image and index. Edit a copy using `set_field`,
   or use `Field::write`. The scalar adapter checks ranges; raw byte arrays
   must match the exact extent. Derived tree edits do not mutate the image.
6. Call `fix_with_context` on the candidate. Repair is transactional and must
   preserve image length. The program controls initialization and copy repair;
   some grammars intentionally have no protection rewrite.
7. Decode again and call `verify_with_context`. Verification returns a map
   of JSON pointers to every `valid`, `checksum_ok`, `signature_valid`, `ok`, `crc_ok` and `checksum_valid`
   boolean. Empty or working-copy flags do not imply a usable save. Check
   the edited record and the program's copy-selection decision.
8. Write the candidate bytes at the application boundary. Preserve a backup;
   an editor can replace its destination atomically. The engine itself returns
   or modifies memory and does not open host files.

## Read and write from any language

Fixed layouts use ordinary JSON, byte arrays and integer arithmetic. A client
can edit a known stored field without implementing the entire semantic runtime:

1. Validate the JSON against the supplied schema, then check cross-field
   invariants: field extents, block indexes, record extents, array geometry and
   transform limits. Reject unknown schema versions or operations.
2. Select the appropriate medium and check its full length against `sizes`.
   Determine active slots/copies using the program's validity decisions.
3. Check `offset <= length` and `size <= length-offset` before slicing. Read
   unsigned scalars with explicit little-endian arithmetic; retain raw bytes
   and reserved regions. For instance, `u16le = byte[o] + 256*byte[o+1]`.
4. To obtain the full semantic JSON tree, initialize and execute `program`
   using the conventions below. This includes text decoding, conditional
   records, checksum coverage and redundant-copy policy.
5. Change only the chosen stored representation in a candidate byte array.
   Check scalar ranges or byte-array length first. For encoded/scrambled fields,
   decode, edit, re-encode with the configured transform and update all copies
   required by the program. Follow record links rather than guessing offsets.
6. Run the repair entry on the candidate, or translate that function's ordered
   writes and checksum expressions. Recompute after editing, observing which
   checks cover other stored checks. Do not blindly fill every copy.
7. Run the decode entry again; verify the edited record, signatures, ranges and
   copy-selection rules, then write exactly the original image length.

### Semantic program conventions

Nodes are JSON objects tagged by `t`. Their property names match the structural
abstract syntax below; the schema gives every allowed node and property.
This is data interpreted by a bounded runtime, with no host imports or I/O.
Values are null, booleans, integers, floats, Unicode strings, byte arrays, lists,
sets and maps. Tuples share list semantics. Integers use checked signed 128-bit
arithmetic, with bounded wide integers for large bit-packed values. An unsupported
operation, missing value, invalid index, zero divisor or overflow returns an
error. Preserve evaluation order and aliasing of mutable lists/bytes/maps.

| Node group | Properties and execution |
| --- | --- |
| `Module` | Execute `body` in order in a root scope; `type_ignores` is empty metadata. |
| `Constant`, `Bytes`, `Name` | `value` (optional `kind` annotation), byte integers `v`, or scope identifier `id`. A missing scope name resolves to a supported native primitive name. |
| `List`, `Tuple`, `Set`, `Dict` | Evaluate `elts`, or paired `keys`/`values`; a null dict key spreads the corresponding map. Sets remove equal duplicate elements. |
| `Attribute`, `Subscript`, `Slice` | Evaluate `value`; select `attr`, or `slice`. Slices use `lower`, `upper`, `step`, null for defaults; indexes can be negative. Positive slices clamp to the sequence length. The bundled runtime's negative-step form reverses the full sequence. |
| `Call`, `keyword`, `Starred` | Evaluate `func`, then `args` left-to-right; `keywords` supply `arg`/`value`; a starred argument expands `value`. |
| `BinOp`, `UnaryOp` | Evaluate `left`/`right` or `operand`, then apply tagged `op`: Add, Sub, Mult, Div, FloorDiv, Mod, BitAnd, BitOr, BitXor, LShift, RShift; Not, USub, Invert. Floor division/remainder are Euclidean for integers. |
| `BoolOp`, `Compare` | Short-circuit `values` with And/Or, returning the selected value. Compare `left` along `ops`/`comparators`; Eq/NotEq, Is/IsNot (value equality), Lt/LtE/Gt/GtE, In/NotIn. |
| `IfExp`, `NamedExpr` | Evaluate `test`, then `body` or `orelse`; or evaluate `value`, assign `target`, return the value. |
| `JoinedStr`, `FormattedValue` | Concatenate `values`; formatted children evaluate `value` and optional `format_spec`. `conversion` is metadata. Supported formatting is decimal/hex/binary/string/fixed-point, width, zero fill, precision and alternate base prefixes. |
| `FunctionDef`, `Lambda`, `arguments`, `arg` | Define a closure using `name`, `args`, `body`; lambda body is an expression. Bind positional `args` (`arg` names), trailing `defaults`, then supplied named arguments. Defaults cannot outnumber parameters. Additional argument metadata is retained; unsupported calling conventions fail. |
| `Assign`, `AnnAssign`, `AugAssign`, `Delete` | Assign `value` to `targets`/`target`, or apply `op` to the target's old value; delete names or indexed entries. Tuple/list targets destructure. |
| `Expr`, `Return`, `Raise`, `Assert` | Evaluate `value`, return it, raise evaluated `exc`, or require truthy `test`; `msg`/`cause` are metadata. A null return is null. |
| `If`, `For`, `While` | Evaluate `test` or `iter`, execute `body`; `For` assigns `target`. Execute `orelse` when no break occurs. Break/Continue transfer loop control. |
| `With`, `withitem` | Evaluate `context_expr`, assign `optional_vars` if present, execute `body`. Virtual streams need no host cleanup. |
| `Try`, `ExceptHandler` | Execute `body`; on error execute first handler `body`, binding error text to `name` if given. Handler `type` is retained metadata. On normal completion execute `orelse`; execute `finalbody` afterward. |
| `Nonlocal`, `Pass` | Assign listed `names` in the enclosing scope; Pass does nothing. |
| Comprehensions | ListComp, GeneratorExp, SetComp evaluate `elt`; DictComp evaluates `key`/`value`. `generators` contain `target`, `iter`, `ifs`, `is_async` (zero). Evaluate nested generators and filters in an isolated child scope; generator expressions are bounded eager lists. |

Native operations include list/set/map construction and methods; range,
enumerate, zip, sum, min/max, sorted/reversed, all/any, next, len, bool, str,
int, isinstance, chr/ord, abs, round, divmod, hex/bin/format; string/byte
join, split, replace, strip, find/index, count, prefix/suffix checks, padding,
hex and byte/integer conversion; Unicode NFC/NFKC; ASCII, UTF-8, UTF-16LE,
Shift-JIS and EUC-JP decoding; packed `struct` reads/writes with explicit
endianness and format widths; SHA-1/SHA-256 and CRC32. Their data operations
follow their ordinary Python-like conventions; reject unsupported formats
rather than silently choosing a platform's native layout. The executable
reference for these adapters is `crates/ws-saves/src/native.rs`.

`definition_checksum(binding, operand)` invokes the configured checksum;
`definition_primitive(binding, value, rounds)` invokes the configured transform.
`input_table(key)` obtains caller-supplied lookup bytes, never ROM bytes embedded
in a definition. `open`, stream read/write and path-existence operations refer
only to a virtual map of the primary and caller-supplied companion images.
`json.dumps` constructs a JSON tree; `print` emits that tree. Diagnostic prose
is discarded. Environment lookups return their supplied default.

After module initialization, invoke the decode entry. `program` mode calls
`main(["save", name])`; `sys-argv` mode calls `main()` with `sys_argv` set to
that vector; `arguments` calls `main([name])`; `direct` invokes `entry.decode`
with `(image_bytes, name)` and emits the returned tree. Require exactly one
JSON result and a false/null/zero main return. Unwrap a singleton outer result
list and remove transport keys named `file` recursively; retain semantic keys,
values and order, including filename-derived keys and `source` values.

Repair uses `["save", "--fix", name, "repaired"]`, with `arguments` omitting
its first item. A direct repair calls `entry.fix(image_bytes)` and uses the
returned bytes. Otherwise retrieve virtual image `repaired`. Check its full
length before replacing the candidate. Application filesystem paths are not
part of the library's capabilities.

Apply limits before allocation: at most 1,048,576 items per image/container,
4 MiB definition text, 50,000,000 semantic operations per invocation and
128 nested calls. Check record multiplication/addition for overflow. Unsupported
native names cannot acquire host capabilities. A failed repair leaves its
input unchanged.

## Worked example: key-mixed cartridge EEPROM

Consider a 128-byte EEPROM definition with unsigned field `ghost_piece` at
byte 0 and a 16-bit checksum at byte 116. Its checksum binding selects the
first 114 bytes as 57 little-endian words, `fold: "xor"`, `bits: 16`,
`seed: 60`, `xor_out: 0`. Bytes 114/115 are a skipped word; bytes 118..127 are an unused tail.
The decode program reports whether the tail is zero.

If the 57 data words are all zero, the computed checksum is 60 (`0x003c`).
Writing `ghost_piece = 1` changes word 0 to 1. The new checksum is
`60 XOR 1 = 61` (`0x003d`), stored as bytes `[61, 0]` at offsets 116/117.
All other bytes, including the skipped word and unused bytes, remain unchanged.

This language-neutral pseudocode performs the stored edit and check update:

```text
require length(image) == 128
candidate = copy(image)
require 0 <= edited_value <= 255
candidate[0] = edited_value
acc = 60
for offset = 0, 2, ..., 112:
    word = candidate[offset] + 256 * candidate[offset + 1]
    acc = (acc XOR word) AND 65535
candidate[116] = acc AND 255
candidate[117] = (acc >> 8) AND 255
run definition's decode; require the edited record's valid flag
write candidate
```

The example is an arithmetic illustration, not a generated save image. For a
real save, use its complete definition's repair function, which may initialize
signatures or synchronize copies as well as updating the checksum.

## Generation and validation

`tools/generate-saves.py` consumes external `format.tsv`, packed `save.h` and
owned `parse_save.py` sources. `--media` supplies external geometry metadata;
`--policy` points to existing JSON definitions whose primitive bindings and
explicit checksum input overrides should be retained. `--output` is the data
directory and `--records` is the shared generated record-width adapter.
Inspect source changes and schema changes together. Runtime embedding discovers
the data files at build time; no parallel game catalog is maintained in Rust.

Frozen natural, repair and valid play expectations remain outside Git. They
compare full normalized value trees and byte digests. Enroll only new namespaces,
then run through Formatkit's serialized runner with an explicit corpus root and
required mode. Ordinary tests use synthetic data with corpus variables unset.
