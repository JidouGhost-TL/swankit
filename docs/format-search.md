# Common-format search

`swankit assets search` scans bytes for **leads**, independently of the existing
evidence-driven `assets discover` command. It does not require Ghidra or an
emulator. The Rust entry point is `ws_discover::search::search`, also re-exported
as `swankit::discover::search::search`.

```sh
swankit assets search cartridge.wsc --output leads.json --json
swankit assets search cartridge.wsc --config search-options.json --output leads.json
swankit assets search-corpus inputs.json --output campaign --jobs 8 --json
swankit assets search-corpus inputs.json --output campaign --jobs 8 --resume
```

Reports contain scalar options and pattern identity/length/hash metadata, not
the pattern bytes or masks. `configuration_sha256` binds the complete original
options, including external bytes; retain that configuration separately to repeat
a search.

Inputs are read without modification. Output files are protected against source
aliases using the existing asset output writer. Keep campaign directories outside
source directories when working with read-only collections. No ROM or decoded
payload bytes are copied into lead reports; external pattern configurations can
contain reference bytes and should be treated according to their provenance.

## Detectors and their limits

| Family | Search rule | What a hit establishes |
| --- | --- | --- |
| `lz-u8-zero` | Literal-first, zero-terminated byte commands | Bounded parse with the existing decoder |
| `lz-rle-7f` | Literal/run-first byte commands, terminating `7f ff` | Bounded parse with the existing decoder |
| `lz-bits-16` | Literal-first, low-bit-first word flags | Bounded parse with the existing decoder |
| `lz-word16-msb` | Counted literals or literal-first high-bit-first word flags | Bounded parse with the existing decoder |
| `lz-word-counted` | A small nonzero command count and valid first literal command | Bounded parse of a word table and separate literal pool |
| `rgb444` | Sixteen aligned little-endian words, upper nibbles zero, at least four different colours | A plausible 32-byte palette window |
| `screen-map` | Thirty-two aligned hardware screen words; at least eight distinct tile indices, at most three palettes, and at least 24 neighbouring index differences at most 32 | A plausible 64-byte map window |
| `tile-map-record16` | A 16-byte descriptor with positive 8-pixel dimensions, a bounded tile count, related tile/map offsets and counts, and valid bounded map words | A complete raw record span satisfying those relations; reader context and meaning remain unknown |
| `reader-signature` | An external exact or per-bit-masked pattern | A byte-pattern match, possibly related reader code |
| `reference-bytes` | An external exact, nonuniform block of at least 16 bytes | A matching reference chunk in ROM; matching decoded chunks are also recorded on codec leads |

Hardware layouts follow [the cited display rules](hardware.md#display-snapshot-rules).
Palette and map windows can be ordinary numeric tables. A decoder can successfully
parse an unrelated byte sequence or start inside another resource. A reader
signature does not establish a common engine or resource-directory grammar.
Reference chunks can have duplicate sources and can match only part of an output.
**No hit establishes a verified asset, whole-output equivalence, resource
ownership, reachable selector domain or safe replacement.**

`tile-map-record16` checks the descriptor and the full tile/map span in the input;
it is not limited by the codec-only `max_stored` or 64 KiB bank window. Its
candidate span is `16 + tile_count * 16 + map_entries * 2` bytes. It charges one
probe per inspected descriptor and one per inspected map word against
`max_probes`. Unknown header words are unconstrained, flip bits are allowed, and
bank or reader context is not inferred. This structural match is not a verified
format or asset.

There is no universal raw-tile, text, font, sample or music signature. Use captured
reference blocks, reader signatures or the existing source/capture discovery
adapter for these. Externally sized and unknown-history codecs are omitted rather
than probed with invented sizes or zero-filled history.

## Configuration

Options are optional, strict JSON: unknown keys are errors. The defaults are:

```json
{
  "stride": 1,
  "max_probes": 64000000,
  "max_stored": 8192,
  "max_decoded": 16384,
  "min_decoded": 64,
  "min_savings": 8,
  "max_command_words": 256,
  "leads_per_family": 64
}
```

The nine original families are enabled by default. `families` can select a subset
from the table above, including `tile-map-record16`. External patterns use
`signatures`:

```json
{
  "families": ["reader-signature"],
  "signatures": [
    {"id": "synthetic-reader", "bytes": [18, 52, 86, 120, 0, 0],
     "mask": [255, 255, 255, 255, 0, 0], "family": "reader-signature"}
  ]
}
```

This example is invented. Reader signatures require at least 32 constrained bits;
lengths are 4–512 bytes. At most 128 patterns are accepted. For captured tile
blocks use `family: "reference-bytes"`, omit the mask, retain source/frame/mode
provenance separately, and exclude uniform patterns. Their family must be enabled.
The decoder matcher records up to 64 matching reference positions per lead and
sets `reference_matches_truncated` when more matches exist. At most two positions
per pattern are retained, so repetition does not crowd out other patterns.
`already_present_in_stored_input` states only whether the exact chunk also occurs
contiguously in that stored span. A decoded-only chunk can be assembled by literal
commands as well as back references; it is not a provenance or codec-use proof.

Every configured byte offset is considered, but codec input windows stop at the
next 64 KiB file-bank boundary and at `max_stored`. Probes use empty initial
history. First-command checks and nonuniform-prefix checks suppress obviously
unsuitable starts. Short/uniform decoded outputs are omitted. By default a parse must save at least
eight bytes compared with its stored extent; set `min_savings` to zero to include
literal-only or expanding framing candidates. This filter reduces false-positive
literal parses but can miss genuine uncompressed/literal resources. Successful
parses rejected by these filters are counted separately. These filters can
miss real resources. A configured complete scan is not exhaustive format coverage.

`max_probes` bounds decoder attempts, direct signature checks,
decoded-reference pattern-search operations, and `tile-map-record16` descriptor
and map-word checks together. Exact patterns use the existing optimized
byte-search mechanism rather than comparing every pattern at every byte. The
report exposes those counters and skipped work.
If the budget prevents a configured check, `complete_for_configured_scan` is false.
Increasing limits or changing stride changes the search; record that configuration.

Candidate totals and retained leads are distinct. The scanner keeps scanning
after a family reaches its retention limit, retaining the highest ranked leads.
Rank is a deterministic sorting weight, **not a probability**. Complete parses,
output size/compression ratio and distinct captured byte patterns affect it.
Decoded-only patterns get a higher ranking weight than patterns already present
in the stored span; repeated positions and duplicate pattern IDs do not multiply
that weight. `overlapping_codec_leads` groups intersecting retained spans and
identifies a preferred ranked lead while preserving all alternatives. These
geometric groups are not proven resource boundaries or independent assets;
full output validation still requires independent evidence.

## Corpus manifests, progress and resume

```json
{
  "schema": "swankit-format-search-inputs/1",
  "inputs": [
    {"id": "first", "path": "roms/first.bin", "sha256": null},
    {"id": "second", "path": "roms/second.bin", "config": "configs/second.json"}
  ]
}
```

Paths are relative to the manifest unless absolute. `sha256`, when supplied, must
match the actual file. Per-input `config` replaces the global `--config`; omitted
options still receive defaults. IDs must be unique. Input size is limited to
64 MiB, manifest size to 8 MiB, configuration size to 1 MiB and concurrency to
1–16 workers. The library does no filesystem I/O or threading.

Each successful input is checkpointed under `leads/` before the batch completes.
Stderr reports progress; `summary.json` records each success, error, incomplete
scan and resumed checkpoint. One failed input does not stop other workers, but
makes the command exit unsuccessfully. Budget-incomplete searches remain explicit
in the summary even when the command completes. Earlier successful checkpoints
are preserved on later failures.

Resume checks executable SHA256, exact manifest binding, input SHA256, options and
report digest. It also refuses a changed ROM at an existing checkpoint index,
including when a prior interruption left no summary. Use a new directory for a
changed executable, input set or configuration. Never interpret a resumed count
as new research. The summary is the latest attempt's status, not an immutable log.

## From leads to resource evidence

1. Preserve the original report, ROM digest, configuration and executable digest.
2. Inspect the most promising offsets in the appropriate Ghidra ROM bank view.
   File offsets are not CPU addresses; recover actual reader/caller mappings.
3. Compare full decoded bytes to a complete original output. A captured tile
   match is partial corroboration and should retain its frame and mode context.
4. Recover framing, lengths, directories, selectors and exceptions from code.
   Treat uncertain source origins and missing history as unresolved evidence.
5. Construct an identity-bound asset map only after these checks; run existing
   map verification and extraction. Do not relocate or edit based on search hits.
6. Refine a general rule or a private signature set, retain its negative controls,
   and run a new campaign. Compare candidate precision and known-reference recall
   separately from counts of leads or already-known resource offsets.

Search reports deliberately contain file offsets, evidence and follow-up needs,
so they can be used alongside Ghidra without making Ghidra a prerequisite for
the bulk scan. No new Ghidra analyzer or automatic code classification is added.
