# ws-text

Private, data-defined character maps, text tables and font strips. All byte
reads use Formatkit's checked readers and fields. Glyph images use ws-gfx's
tile renderer and the shared `formatkit_core::Image` type.

Load an external `swankit-text-def/1` JSON definition with
`TextDefinition::from_json`, or select a reviewed neutral ID with
`builtin_definition`. Compile its map, then decode a bounded byte slice.
`DecodedText` contains raw cell text, normalized text, consumed bytes,
termination status and source tokens. `encode_tokens` preserves original
aliases, attributes and invisible page changes. `encode` creates a canonical
encoding of raw cell text and validates it by decoding again. `insert_text`
checks the complete encoding and slot before changing any resident bytes.

`StringTableDefinition` supports explicit offsets, fixed records, integer
pointers and segment-window far pointers. `FontDefinition` describes tile
formats, strip strides, tile order, sparse half-width storage and optional
advance tables. The caller supplies the palette and resource limits.

See [text definitions](../../docs/text-definitions.md) for the JSON/TSV schema,
worked read/write examples, encoding policies and limitations, and
[definition provenance](data/README.md) for the bundled data.

Synthetic verification:

```sh
tools/ci.sh
```

Real strings, images, expectations and population counts stay in an external
manifest. Enroll independent oracle output into new selections only:

```sh
python3 tools/stage-corpus.py --text-manifest "$TEXT_MANIFEST" --output "$STAGING_DIR"
python3 tools/stage-corpus.py --text-manifest "$EXTRA_MANIFEST" \
  --text-spec-dir ws-text-extra --output "$STAGING_DIR"
FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

The `ws-text` and `ws-text-extra` corpus selections are required together in a
deliberate corpus run. The extra selection covers independently referenced
pointer tables, an unterminated oracle prefix and unknown-control diagnostics.
The corpus handler checks each dumped string against independent text, exact
source-token encoding and readable encode/decode, and each font bank against
RGBA bytes from independent image oracles. Corpus support is dev-only.

No interpreter discovers scripts by scanning a cartridge. Supply independently
reviewed starts or table metadata. Unknown glyphs must have an explicit table
entry; unknown controls and truncated operands return errors. Text insertion
does not relocate pointers, update checksums, emulate interpreter effects,
measure layout fit or claim an inserted script works in a game.
