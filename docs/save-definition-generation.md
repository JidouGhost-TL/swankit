# Save definition generation

`tools/generate-saves.py` bootstraps portable parser definitions from research
sources. Bootstrap output is preliminary and is not a replacement for reviewed
indexed definitions. Bare layout sizes default to decimal; source conventions
must never be inferred from whether a token happens to parse.

For reviewed field metadata repair, use the existing canonical definitions as
`--policy`, with `--layout-only` and an external `--layout-audit` path:

```sh
python3 tools/generate-saves.py --layout-only --source SOURCE \
  --policy CANONICAL_DEFINITIONS --output NEW_DEFINITIONS \
  --layout-audit AUDIT.json
```

This internal maintenance mode preserves every nonfield value, retains existing
field indices, and appends newly approved fields. Versioned source conventions
and approved packed-member bindings live in
`crates/ws-saves/source-layout-policies.json`. Table and header hashes bind the
review to exact source bytes. Widths and offsets must agree with the packed
members; unapproved differences remain in the external audit. Source drift,
ambiguous bindings, or missing canonical references refuse repair. Unreviewed
sources retain decimal interpretation and must match their indexed fields.

Bootstrap refuses protected source identities, recognized source pins, and
existing protected output definitions. Updating a reviewed source policy requires
new evidence and review, rather than falling back to bootstrap generation.
