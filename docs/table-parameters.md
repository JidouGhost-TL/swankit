# Conditional table source parameters

`swankit assets resolve-table` resolves supplied source coordinates into exact
file slices using `ws_rom::RomGeometry`, mapper-normalized bank registers and
`address_to_file_offset`. It does not recognize code, infer source registers,
observe invocation or assign a media/resource identity. Hardware rules follow
the [memory-map and mapper documentation](hardware.md).

```sh
swankit assets resolve-table cartridge.wsc parameters.json \
  --output receipt.json --map-out candidate-map.json --json
swankit assets extract cartridge.wsc candidate-map.json extracted --json
```

The strict definition has schema `swankit-table-parameters/1`. Supply `rom` with
exact `sha256` and byte `size`, `source_segment` and `source_base` as unsigned
16-bit JSON integers, a `route`, and a `context`. The segment is the explicitly
known selected source-segment value (ES for the indexed-copy reader); an unknown
DS/SS/ES value cannot be inferred from a code-family match or the destination.
Offsets/sizes inside `rom` and retained file coordinates also accept `0x` strings.

An invented parameter example (replace the identity with the original image):

```json
{
  "schema": "swankit-table-parameters/1",
  "rom": {"sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 2097152},
  "source_segment": 20771,
  "source_base": 9029,
  "route": {"kind": "record", "entry_al": 255},
  "context": {"kind": "explicit", "banks": [{"c0": 1, "c2": 0, "c3": 0}]}
}
```

`record.entry_al` is the byte offset already present at core entry: it is added
directly to the base, without multiplication. It selects 16 bytes. The
`aligned-fallthrough` route instead describes the possible aligned starts
0,16,...,240 after an eight-bit left shift by four, a contiguous 256-byte domain.
An alternate entry may bypass that shift. `core-union` describes starts 0..255,
a 271-byte union of possible reads. Neither union identifies one named resource.
Row order is preserved; equal row multisets do not make indices interchangeable.

`explicit.banks` contains one to 64 possible complete C0/C2/C3 states. Each is
normalized for the footer's mapper and duplicate normalized states are collapsed.
Alternatively, `context` can be `{"kind":"retained-code","segment":16384,
"offset":4096,"file_offset":"0x141000"}`. This enumerates all compatible
implemented C0 values. Both the recorded code address and source must then lie
in C0-backed CPU windows at or above `0x40000`; C2/C3 sources require explicit
registers. Inferred receipts leave C2/C3 unbound (`null`), rather than claiming
zero register values. A retained logical code coordinate and stable bank context
are conditions, not execution evidence.

The versioned `swankit-table-resolution/1` receipt includes the original
parameters, normalized compatible contexts, 16-bit SI range, 20-bit CPU range,
effective chip geometry/padding, distinct file offsets, byte lengths and SHA-256
for each candidate. An ambiguous receipt succeeds as a report with
`decision:"ambiguous"`; it cannot emit a single map. `--map-out` requires one
distinct contiguous file slice. That map uses existing `Codec::Raw` and `other`
output; no new Raw codec selector or editing/permission policy is introduced.
Normal map editing rules still apply. Source-domain evidence alone does not
establish ownership or safe editing.

The initial resolver refuses SI wrap, CPU mapping-window crossings, RAM/SRAM,
unbacked start padding, unknown mapper families and changed original identity.
Segment paragraphs are included before wrapping the physical CPU address to
20 bits; a wrap can therefore land in RAM and be refused. Effective chip offsets
are converted to file offsets rather than reported as file positions. Output
sets use the existing atomic preflight/overwrite safeguards (`--force` permits
overwriting output files, never inputs).

## Flat fixed-stride offset tables

The same command accepts `swankit-fixed-stride-table/1` for a reviewed table
of little-endian 32-bit offsets relative to a known data window. All fields
outside the ROM identity are JSON numbers. Offsets and sizes are in bytes
unless a configured multiplier says otherwise.

```json
{
  "schema": "swankit-fixed-stride-table/1",
  "rom": {"sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 65536},
  "table_offset": 0,
  "entry_count": 2,
  "entry_stride": 8,
  "offset_field": 0,
  "offset_unit": 1,
  "data_offset": 256,
  "data_length": 1024,
  "size": {"kind": "explicit-u32", "field_offset": 4, "unit": 1},
  "padding_min_bytes": 8
}
```

`explicit-u32` reads an additional little-endian length from every row.
`{"kind":"sequential-until","sentinel_end":1024}` instead takes lengths from
successive offsets. The optional sentinel is relative to the data window and
defaults to its length. Sequence offsets must be nondecreasing.

The receipt includes absolute file offsets and retains zero-length rows. A Raw
map can be emitted for disjoint nonempty ranges. With `padding_min_bytes` set,
gaps of at least that size are reported if they are all zero, a repeated byte,
or a repeated little-endian 32-bit word. These gaps are inspection candidates.
The generated map leaves free-space declarations empty. Use the banked
definition above for CPU segment and mapper coordinates.
