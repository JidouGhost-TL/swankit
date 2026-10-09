# ws-patch

Checked IPS parsing, validation, application, serialization and creation.
All byte reads use `formatkit_core::Reader`. The owner catalog descriptor is
`catalog::IPS_FORMAT` (`ws-patch-ips`); corpus handlers are dev-only.

```rust
use ws_patch::{create, ApplyOptions, Patch};

let base = b"012345";
let target = b"01abcd";
let encoded = create(base, target)?;
let patch = Patch::parse(&encoded)?;
let validation = patch.validate(base.len(), ApplyOptions::default())?;
assert_eq!(validation.output_size, target.len());
assert_eq!(patch.apply(base, ApplyOptions::default())?, target);
# Ok::<(), formatkit_core::Error>(())
```

The [ZeroSoft IPS specification](https://zerosoft.zophar.net/ips.php)
defines PATCH, 24-bit big-endian offsets, 16-bit big-endian lengths, literal
and nonzero RLE records, and EOF. The optional three-byte size following EOF
is documented by
[EZ ROM Patcher's format documentation](https://ezrompatcher.com/formats/ips).
This crate treats that extension as the exact final size: records apply first,
then the image shrinks or zero-extends. Inputs with any other trailing extent,
zero-length RLE, incomplete records or a missing EOF return errors.
No hardware rules are introduced by this image-independent format.

Parsing retains record order and accepts overlaps as grammar. Validation and
application default to rejecting overlaps and growth. Callers may explicitly
select `OverlapPolicy::LastWriteWins` and `GrowthPolicy::ZeroFill`. The output
limit defaults to 64 MiB and bounds the base, record extents and final size,
including writes later discarded by truncation. Validation finishes before
output allocation; apply never mutates the base. Record metadata is bounded
by `parse_with_record_limit` (one million records by default). Literal payloads
borrow the patch and RLE records stay compact. A record can end beyond the
24-bit offset ceiling if its start and length are representable.

`Patch::encode` preserves each record's representation, order and EOF size.
It bounds serialized bytes at 64 MiB before writing; callers can override this
with `encode_with_size_limit`. Creation also bounds metadata at one million
records, returning a resource error if a fragmented diff exceeds that limit.
`create` skips unchanged spans and uses RLE for constant spans when smaller
than a literal. It splits long spans at 65535 bytes and includes the preceding
byte when a record would otherwise start at reserved offset 454F46 (EOF).
The resulting diff has minimal changed spans; it does not globally optimize
compressed size by merging gaps or extracting interior runs. Creation supports
targets up to 16777215 bytes; optional size extensions handle shrinkage and
zero-filled growth. IPS carries no base checksum: callers must verify base
identity externally. BPS is outside this milestone.

Enroll an independent external manifest once:

```sh
python3 tools/stage-corpus.py --patch-manifest "$PATCH_MANIFEST" --output "$STAGING_DIR"
FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

The manifest's samples specify `name`, `source`, `base`, `target`, `expect`
(`sha256`, `base_sha256`, `patched_sha256`, `base_size`, `patched_size`,
`records`), and `provenance` (`reference_kind`, nonempty `references` of paths
and hashes). Targets must come from independent reference material, never
this crate. Staging checks every input and reference hash before writing a
new spec; it refuses to overwrite a frozen selection. Its namespace-local
enrollment ledger preserves duplicate patch contents as separate evidence.
The corpus test compares application and create/apply results with independent
target bytes and frozen digests, and checks parse/encode byte preservation.
