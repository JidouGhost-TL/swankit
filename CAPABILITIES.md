# swankit Capabilities

Generated from `swankit capabilities --json` (`formatkit.capabilities.v1`).

- **Tool:** `swankit` `0.1.3`
- **FormatKit:** `0.1.14`
- **Save engines:** `swankit-save-def/1`, `swankit-save-def/2`
- **Patch formats:** `ips`

## Supported Formats

| Format ID | Category | Tier | Capabilities | Notes |
| --- | --- | ---: | --- | --- |
| `ws-rom` | `rom` | 2 | `parse`, `decode` | WonderSwan and WonderSwan Color cartridge footer, metadata, banked address mapping, and checksum verification |
| `ws-lz-rle-7f` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | Bounded LZ/RLE 0x7F stream decoder and deterministic encoder |
| `ws-lz-bits-16` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | 16-bit flag LZ stream decoder and deterministic encoder |
| `ws-lzss-12` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | 12-bit distance LZSS stream decoder and deterministic encoder |
| `ws-lzss-u16-zero` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | Zero-terminated 16-bit flag LZSS stream decoder and deterministic encoder |
| `ws-okumura-lzss` | `compression` | 1 | `parse`, `decode`, `reproduce`, `author` | Okumura-style ring-buffer LZSS stream decoder and deterministic encoder |
| `ws-lz-u8-zero` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | Zero-terminated 8-bit flag LZ stream decoder and deterministic encoder |
| `ws-lz-word16-msb` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | MSB 16-bit word-flag LZ stream decoder and deterministic encoder |
| `ws-lz-word-counted` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | Word-counted LZ stream decoder and deterministic encoder |
| `ws-lzss-12-ext` | `compression` | 2 | `parse`, `decode` | External-length 12-bit LZSS stream decoder |
| `ws-flag16-distance11` | `compression` | 2 | `parse`, `decode` | 16-bit flag, 11-bit distance LZ stream decoder |
| `ws-mask16-fill` | `compression` | 2 | `parse`, `decode` | 16-bit mask fill stream decoder |
| `ws-lz-command16-lsb` | `compression` | 2 | `parse`, `decode` | LSB 16-bit command LZ stream decoder |
| `ws-lzss-msb-distance12` | `compression` | 2 | `parse`, `decode` | MSB 12-bit distance LZSS stream decoder |
| `ws-lzss-be-units` | `compression` | 2 | `parse`, `decode` | Big-endian unit-framed LZSS stream decoder with provenance |
| `ws-rle-77ee` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | Escape-byte word and byte RLE stream decoder and deterministic encoder |
| `ws-rle-pcm-00` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | Zero-escaped byte RLE stream decoder and deterministic encoder for 8-bit PCM |
| `ws-counted-word-lz` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | Word-aligned counted-command LZ stream decoder and deterministic encoder |
| `ws-dpcm-6bit` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | 6-bit packed signed DPCM audio stream decoder and deterministic encoder |
| `ws-lzss-11-5` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | 2 KiB ring-buffer 11-bit position and 5-bit length LZSS stream decoder and encoder |
| `ws-planar-rle` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | Bitplane-split 0xFFFF-terminated tile RLE stream decoder and deterministic encoder |
| `ws-lzss-token-be` | `compression` | 2 | `parse`, `decode`, `reproduce`, `author` | Big-endian token-counted bit-flag LZSS stream decoder and deterministic encoder |
| `ws-tile-2bpp` | `gfx` | 2 | `parse`, `decode`, `reproduce`, `edit` | 2bpp planar 8x8 tiles (monochrome and 4-color palettes) |
| `ws-tile-4bpp-planar` | `gfx` | 2 | `parse`, `decode`, `reproduce`, `edit` | 4bpp planar 8x8 WonderSwan Color tiles |
| `ws-tile-4bpp-packed` | `gfx` | 2 | `parse`, `decode`, `reproduce`, `edit` | 4bpp packed 8x8 WonderSwan Color tiles |
| `ws-mono-palettes` | `gfx` | 2 | `parse`, `decode` | Monochrome shade lookup tables and 8-palette register decoding |
| `ws-color-palettes` | `gfx` | 2 | `parse`, `decode` | 12-bit BGR444 color palette tables (16 palettes x 16 colors) |
| `ws-screen-map` | `gfx` | 2 | `parse`, `decode` | 32x32 16-bit background screen map entries |
| `ws-sprite-table` | `gfx` | 2 | `parse`, `decode` | 32-bit OAM sprite table entries |
| `ws-display-snapshot` | `gfx` | 2 | `parse`, `decode` | 64 KiB RAM + 256-byte I/O port display snapshot renderer (hardware and diagnostic profiles) |
| `ws-character-map` | `text` | 2 | `parse`, `decode`, `reproduce`, `edit` | Data-defined character maps (swankit-charmap/1) with control codes and shift pages |
| `ws-text-table` | `text` | 2 | `parse`, `decode`, `reproduce`, `edit` | Definition-driven fixed, delimited, and pointer-table string extraction and re-encoding |
| `ws-font-strip` | `text` | 2 | `parse`, `decode` | Bitmap tile font atlas extraction from ROM definitions |
| `ws-sound.wave` | `audio` | 2 | `parse`, `decode` | 4-bit packed hardware wave RAM banks and channel waveforms |
| `ws-sound.unsigned-pcm` | `audio` | 2 | `parse`, `decode` | 8-bit unsigned hardware PCM sample streams and 24 kHz WAV preview rendering |
| `ws-sound.signed-pcm` | `audio` | 2 | `parse`, `decode` | 8-bit signed PCM sample streams and WAV rendering |
| `ws-sound.byte-sequence` | `audio` | 2 | `parse`, `decode` | Byte-oriented sound driver sequence decoding and preview rendering |
| `ws-sound.cell-sequence` | `audio` | 2 | `parse`, `decode` | Cell-oriented sound driver sequence decoding and WAV/event rendering |
| `ws-sound.sdk-sequence` | `audio` | 3 | `parse`, `decode` | SDK sound driver bank discovery, sequence decoding, and hardware-matched WAV/event rendering |
| `ws-patch-ips` | `patch` | 2 | `parse`, `decode`, `reproduce`, `edit`, `author` | IPS binary patch creation, validation, and policy-checked application with RLE and EOF truncation |
| `ws-save-image` | `save` | 2 | `parse`, `decode`, `edit` | Cartridge SRAM/EEPROM and internal EEPROM media geometry and checksum verification |
| `ws-defined-save` | `save` | 2 | `parse`, `decode`, `reproduce`, `edit` | Declarative save definitions (swankit-save-def/1, swankit-save-def/2) for field decode, checked edits, and transactional repair |
| `ws-asset-map` | `assets` | 2 | `parse`, `decode`, `reproduce`, `edit` | Per-ROM declarative asset maps for verification, extraction, checked replacement, relocation, and discovery |
| `ws-link-relay` | `link` | 2 | `parse`, `decode`, `reproduce` | Unix serial byte-stream relay across TCP, stdio, exec, raw device, and WonderGate endpoints |
| `ws-wondergate` | `wondergate` | 3 | `parse`, `decode`, `reproduce`, `author` | WonderGate adapter framing, excomm adapter, wire tap decoder, and local HTTP/BPL service emulation |
