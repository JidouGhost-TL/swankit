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

The optional configuration field `budget_scope` accepts `"global"` (default)
or `"per-family"`. Global keeps the established shared `max_probes` ceiling
and detector order. Per-family repeats the configured scan independently for
each enabled family, granting each its own `max_probes` ceiling; total work can
therefore increase with the number of enabled families. Decoded reference
comparisons charge the codec family's allowance, while ROM reference searches
charge the reference family's allowance. Retention stays bounded independently
by `leads_per_family`. No family receives an unbounded scan.

`complete_by_family` explicitly reports completion for every configured family;
`complete_for_configured_scan` requires all of them to complete. Completion
covers only the configured stride, predicates and bounds, and does not assert
asset coverage. `positions_scanned` counts configured ROM offsets, rather than
the sum of repeated family traversals. A family without an applicable probe
can complete even when another family exhausts its budget.

Counted-directory leads include `directory_provenance`: `base`, `group_length`
(including the directory), `entry_count`, zero-based `entry_index`,
`distinct_records`, and `target_entry_index` (the first entry sharing that
record target). These witnesses distinguish inferred groups and aliases.
Equivalent record spans can be grouped by lead offset/length without deleting
the distinct directory occurrences; occurrence counts remain unchanged.

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
| `sgd1-tile-grid` | Exact `SGD1` tag; positive byte dimensions; little-endian total-size word at +14 equals 16-byte header plus 16 bytes per grid cell; full span fits input | A bounded record matching the tag, declared-size and byte-grid relation; field meanings remain unknown |
| `tile-map-record16` | A 16-byte descriptor with positive 8-pixel dimensions, a bounded tile count, related tile/map offsets and counts, and valid bounded map words | A complete raw record span satisfying those relations; reader context and meaning remain unknown |
| `counted-relative-directory16` | A little-endian relative-u16 directory whose entry count is inferred from its minimum offset, followed by positive 16-byte-unit counted records that form a contiguous span | Individual bounded raw records from a directory-shaped group; payload meanings and table use remain unknown |
| `reader-signature` | An external exact or per-bit-masked pattern | A byte-pattern match, possibly related reader code |
| `banked-queue-constructor` | Explicit opt-in fixed-register instruction core with equal queue-pointer literals | A code-pattern lead retaining literal operands; instruction boundaries, callers and data remain unproven |
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

`sgd1-tile-grid` checks the exact four-byte `SGD1` tag, positive unsigned-byte
dimensions at +4/+5, and the little-endian total-size field at +14 against
`16 + width * height * 16`. Bytes +6 through +13 are unconstrained. The full
declared span must fit in the ROM input; there is no codec `max_stored` or
64 KiB bank-window cap. A following tag is not required. The rule establishes
only the bounded byte relation, not dimension semantics, palette, reader,
runtime use or asset identity.

`counted-relative-directory16` infers the number of u16 relative offsets from
the minimum value among the first N entries: that minimum must equal `2 * N`,
the directory byte length. The first entry may point anywhere in the record
run. Each distinct target must begin with a positive little-endian u16 unit
count and contain exactly `2 + 16 * count` bytes. Sorted distinct records must
start immediately after the directory and continue without gaps or overlap;
repeated directory targets remain repeated leads. Only the directory words
are interpreted as offsets, never record payload words. Group validation checks
each referenced span against the ROM input bounds regardless of `max_stored`;
only records whose individual spans fit `max_stored` are emitted, so one larger
record does not suppress smaller members of a valid group. Every directory word
and distinct record header inspected consumes the shared `max_probes` budget. If that budget stops
the scan, `complete_for_configured_scan` is false. This is a structural
candidate rule, not evidence of payload encoding, resource meaning, a reader,
runtime use, palette or asset identity.

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
from the table above, including the opt-in `sgd1-tile-grid` and
`tile-map-record16` and `counted-relative-directory16`. External patterns use
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
and map-word checks, plus each `sgd1-tile-grid` tagged-header attempt and each
`counted-relative-directory16` directory-word/record-header check, together.
Exact patterns use the existing optimized
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

## Opt-in constructor code pattern

The `banked-queue-constructor` family is explicitly selected with
`{"families":["banked-queue-constructor"]}`. It is absent from the default family
list and does not change empty `reader-signature` configurations. It uses the
existing lead/report fields and remains independent of caller-flow
analysis.

The rule checks one 73-byte, fixed-register instruction-encoding core. It permits
only literal table, initial ES segment and repeated queue-pointer operands to
vary; the two queue-pointer literals must agree. Leads retain the literal operand
values, actual full-span hash, bank-word/AL distinction, OUT port, pointer-window
segment and fixed register roles. Changing an ES override, output port, index
operation or queue-store role refuses the pattern. Instruction encodings are
based on the Intel 8086 instruction reference; hardware bank context is described
in [hardware.md](hardware.md).

These are **code-pattern leads**, including when the bytes have no proven
instruction boundary. A hit does not identify a containing function, caller,
selector, mapped bank value, table extent, decoder, SDK, asset or runtime use.
Literal ES values are recorded rather than replaced with an assumed segment;
no table bytes are dereferenced and no file-bank normalization is performed.
The retained source analysis has richer flow facts that this ROM-only search
cannot ingest: register-lane writes, original instruction/function identity,
direct targets and every recorded incoming branch target. No caller constants
are propagated by this rule. Unknown incoming entries/clobbers remain unresolved;
unvisited code is preserved. Full flow-aware integration needs a separately
reviewed source-facts input contract.

The existing stride, shared probe budget and per-family retention limit apply.
Counts describe configured byte-pattern candidates, not supported games or
assets; a configured zero is not proof of absence. `max_stored` is a decoder/raw
resource bound and does not truncate this fixed code-inspection span.

## Optional retained-source caller evidence

An explicitly configured `banked-queue-constructor` search can additionally read
an external, identity-bound instruction listing:

```sh
swankit assets search image.bin --config constructors.json \
  --retained-source source.json --output leads.json --json
```

This writes `leads.json` and `leads.callers.json`. Alternatively,
`--provenance-output FILE` chooses the sidecar path; it is required when no
`--output` is supplied. The automatic sidecar locator is relative to the report's
directory. An explicitly supplied locator is preserved as supplied, relative to
the invocation directory unless absolute. Both outputs refuse input overlap or
existing files. No attachment means no new report fields and no caller analysis;
defaults, external signatures and ordinary search results remain unchanged.

A batch input can use an optional `retained_source` path alongside `id`, `path`,
`sha256` and `config`. Its path is relative to the manifest. Sidecars are stored
under `provenance/` and referenced by the constructor code leads and their file
envelope. Resume verifies original/configuration/executable/report identities,
the attachment and listing hashes, and recomputes caller evidence before accepting
the sidecar. Changing caller values and updating stored hashes is insufficient.
Missing, partial, changed or symlinked sidecars are refused; no fallback is used.
A missing listing is an explicit failed batch input, with its failure in the
summary. A malformed attachment is a configuration error.

The strict attachment format is `swankit-retained-source/1`. For example, this
invented geometry describes a 64-KiB file with equal logical and file offsets:

```json
{
  "schema": "swankit-retained-source/1",
  "rom_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "rom_bytes": 65536,
  "listing": {
    "path": "listing.tsv",
    "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
    "bytes": 1234
  },
  "mapping": [{"logical_start": 0, "physical_start": 0, "length": 65536}]
}
```

Replace the invented identities and byte count with independently reviewed
originals. A mapping binds linear listing addresses to file bytes; it does not
infer bank residency. Logical ranges must be disjoint and all mapped bytes must
match the original. Instructions outside the reviewed mapping remain unverified
and cannot supply caller arguments. Unknown fields are errors. An optional
`bank_policy: "rom1-low-byte-power-of-two"` enables pointer leads only for an
explicitly reviewed low-byte ROM1 policy and a power-of-two, 64-KiB-aligned file
of at most 16 MiB. The full bank word and its output low byte remain distinct.
No policy is selected automatically, and no runtime bank state is established.

The listing is the existing tab-separated function/instruction export, without
paths or resource payloads added to it:

- `F`: entry in hexadecimal, segmented label, name, decimal instruction count,
  call-reference count, hexadecimal body minimum and maximum.
- `I`: hexadecimal address, containing function entry or `-`, flow name, complete
  instruction hex, same-length mask hex, registers read, registers written,
  assembly text and comma-separated hexadecimal direct targets or `-`.

Include the complete retained export, including unowned instructions and other
functions, rather than only the chosen caller. All recorded direct targets can
invalidate a reaching definition. Counts/membership, byte boundaries, mapped
original bytes and global targets are checked; masks never replace literal
operand identity. The exporter gives parent-register effects, so byte registers
are conservatively normalized to their 16-bit parents. The supported transfer
rules recognize immediate word/byte lanes and XOR-self. Calls, branches, returns,
gaps and recorded incoming entries reset state. Memory/computed writes and
implicit multiply/divide clobbers kill values; unsupported effects kill all
tracked values. Supported instruction effects require an exact original opcode
extent; a longer row cannot hide a second write behind harmless effect metadata.
There is no arithmetic folding, default-zero high lane, branch
join or assumed callee preservation. A recorded entry inside an instruction
prevents source qualification of that function. A function with multiple matching
constructor cores remains ambiguous.

The separately hash-bound `swankit-retained-callers/1` sidecar records constructor
core versus function entry, complete retained function byte identity, direct
calls, original blocks, producer locations for both register lanes, explicit
unknown reasons and optional source-pointer leads. It retains the global recorded
direct-entry edges, unresolved flow sites, original segmented function identities
and, when available, the original instruction that caused a value to be held.
Only constructor-family leads
receive references; external reader signatures retain their previous meaning.
Code spans stay code spans, and no decoded output length is invented.

Caller arguments describe values at the constructor's function entry. They are
not automatically values at the matched core. The constructor's
`entry_to_core_hold` is null only when the retained entry reaches that core
through exact, contiguous, mapped one-byte PUSH/NOP instructions or two-byte
immediate-port input to AL, with no
recorded alternate entry into the prefix or core start. Other prefixes remain held;
the checker does not simulate register rewrites or assume callee preservation.
A held prefix preserves the original code lead, listing compatibility and
caller-entry argument facts, while blocking downstream pointer resolution
before directory bytes are read. An unmapped or otherwise incompatible function
still cannot bind a caller. Even a transparent prefix proves only the retained
direct entry-to-core path, with the incomplete-listing limits below.
The input-to-AL rule proves only that DX/BX/CX are preserved and that the matched
core overwrites AX before use; it makes no port-behavior or bank-state claim.
This interpretation is identified by sidecar policy
`immediate-lanes-neutral-constructor-prefix-v2`.

Assurance is explicitly **retained-direct-flow candidate; incomplete-listing**.
Hash binding is not authentication or proof that the listing is globally correct.
Unknown indirect/unexported entries remain possible. Function envelopes are not
complete program boundaries; matching the retained listing does not adopt a
project or prove execution. No code is discarded. Pointer leads prove neither
object start/extent, a reader's length/bias contract, a codec, assets nor runtime
output. These require separate original-source and complete-output evidence.
