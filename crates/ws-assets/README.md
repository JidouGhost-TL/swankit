# ws-assets

Per-ROM asset maps (`swankit-asset-map/1`, see `docs/asset-maps.md`): a JSON
table of located assets read against one ROM identified by SHA-256.

- `extract`: every asset to PNG (tiles, tilemaps, palettes, fonts, screens),
  UTF-8 text plus raw codes, WAV (PCM) or raw bytes, and an index.
- `replace`: re-encode an edited asset with its codec; write in place when it
  fits, relocate into declared free space when every reference is known,
  otherwise refuse. The footer checksum is always recomputed.
- `verify`: decoding, ranges, declared overlaps, free space fill and
  reference values, with coverage statistics.

Grammars stay in their owner crates (`ws-gfx`, `ws-compress`, `ws-text`,
`ws-sound`, `ws-rom`); this crate holds placement policy only. Maps carry
title labels; the crate itself is title independent.
