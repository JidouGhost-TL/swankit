# ws-saves

A generic engine interpreting one JSON definition per game in `data/`.
Definitions use `swankit-save-def/1` or `/2`; no per-game Rust code is required.
See [the format, read/write instructions and worked example](../../docs/save-definitions.md)
and [JSON Schema](../../docs/save-definition.schema.json).

Load external data with `Format::from_json`, or select a bundled definition using
`format(id)`. Decode to a JSON value tree, select a stored field by its index for
`set_field`, inspect flags with `verify`, and repair transactionally with `fix`.
`DecodeContext` supplies required lookup tables and companion images explicitly.
The library has no host I/O; application code owns reading and writing files.

`recognize_with_context` requires an eligible positive `valid` or `global_valid`
decision whenever the included decoded tree contains either explicit boolean
decision. An ancestor or sibling signature cannot override explicit invalidity;
a positive overall program decision remains authoritative. A region with
`valid: false` or `global_valid: false` cannot contribute usable flags from itself or its
descendants. A separate valid record may still qualify, and named validity
exclusions omit working regions from both positive and negative evidence. A root
`valid: false` or `global_valid: false` vetoes recognition even if excluded.
Without included explicit decisions, positive signatures qualify; passing
checksums alone qualify only when there are no included signature flags either. Diagnostic
counts still include all included checks, even those in unusable records.
`fix` remains a low-level mechanical repair; callers must check usability before
writing its result.

The generator consumes external field tables, packed records and owned semantic
sources. Shared record adapters and all binary reads/edits use Formatkit.
The only native operations are generic primitives whose parameters live in JSON.
