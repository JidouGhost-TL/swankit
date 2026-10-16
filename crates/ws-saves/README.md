# ws-saves

A generic engine interpreting one JSON definition per game in `data/`.
Definitions use `swankit-save-def/1`; no per-game Rust code is required.
See [the format, read/write instructions and worked example](../../docs/save-definitions.md)
and [JSON Schema](../../docs/save-definition.schema.json).

Load external data with `Format::from_json`, or select a bundled definition using
`format(id)`. Decode to a JSON value tree, select a stored field by its index for
`set_field`, inspect flags with `verify`, and repair transactionally with `fix`.
`DecodeContext` supplies required lookup tables and companion images explicitly.
The library has no host I/O; application code owns reading and writing files.

`recognize_with_context` requires a positive semantic validity flag when the
decoded tree includes such flags. A region with `valid: false` or
`global_valid: false` cannot contribute usable flags from itself or its
descendants. A separate valid record may still qualify, and named validity
exclusions omit working regions. Passing checksums alone qualify only when
the included tree has no semantic validity flags. `fix` remains a low-level
mechanical repair; callers must check usability before writing its result.

The generator consumes external field tables, packed records and owned semantic
sources. Shared record adapters and all binary reads/edits use Formatkit.
The only native operations are generic primitives whose parameters live in JSON.
