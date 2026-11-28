use serde::Serialize;

pub const BUILD_IDENTITY_SCHEMA: &str = "formatkit.build-identity.v1";
pub const CAPABILITIES_SCHEMA: &str = "formatkit.capabilities.v1";

#[derive(Debug, Clone, Serialize)]
pub struct BuildIdentity {
    pub schema: &'static str,
    pub tool: &'static str,
    pub version: &'static str,
    pub git_commit: Option<&'static str>,
    pub git_dirty: bool,
    pub formatkit_version: &'static str,
    pub target: &'static str,
    pub profile: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormatCapability {
    pub id: &'static str,
    pub category: &'static str,
    pub capabilities: &'static [&'static str],
    pub tier: u8,
    pub notes: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilitiesManifest {
    pub schema: &'static str,
    pub tool: &'static str,
    pub version: &'static str,
    pub formatkit_version: &'static str,
    pub formats: &'static [FormatCapability],
    pub save_engines: &'static [&'static str],
    pub patch_formats: &'static [&'static str],
}

pub fn build_identity() -> BuildIdentity {
    let raw_commit = env!("SWANKIT_GIT_COMMIT");
    let git_commit = if raw_commit.is_empty() {
        None
    } else {
        Some(raw_commit)
    };
    let git_dirty = env!("SWANKIT_GIT_DIRTY") == "true";

    BuildIdentity {
        schema: BUILD_IDENTITY_SCHEMA,
        tool: "swankit",
        version: env!("CARGO_PKG_VERSION"),
        git_commit,
        git_dirty,
        formatkit_version: env!("SWANKIT_FORMATKIT_VERSION"),
        target: env!("SWANKIT_TARGET"),
        profile: env!("SWANKIT_PROFILE"),
    }
}

static FORMATS: &[FormatCapability] = &[
    // Cartridge ROM (ws-rom)
    FormatCapability {
        id: "ws-rom",
        category: "rom",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "WonderSwan and WonderSwan Color cartridge footer, metadata, banked address mapping, and checksum verification",
    },
    // Compression streams (ws-compress)
    FormatCapability {
        id: "ws-lz-rle-7f",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Bounded LZ/RLE 0x7F stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lz-bits-16",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "16-bit flag LZ stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lzss-12",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "12-bit distance LZSS stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lzss-u16-zero",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Zero-terminated 16-bit flag LZSS stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-okumura-lzss",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 1,
        notes: "Okumura-style ring-buffer LZSS stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lz-u8-zero",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Zero-terminated 8-bit flag LZ stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lz-word16-msb",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "MSB 16-bit word-flag LZ stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lz-word-counted",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Word-counted LZ stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lzss-12-ext",
        category: "compression",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "External-length 12-bit LZSS stream decoder",
    },
    FormatCapability {
        id: "ws-flag16-distance11",
        category: "compression",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "16-bit flag, 11-bit distance LZ stream decoder",
    },
    FormatCapability {
        id: "ws-mask16-fill",
        category: "compression",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "16-bit mask fill stream decoder",
    },
    FormatCapability {
        id: "ws-lz-command16-lsb",
        category: "compression",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "LSB 16-bit command LZ stream decoder",
    },
    FormatCapability {
        id: "ws-lzss-msb-distance12",
        category: "compression",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "MSB 12-bit distance LZSS stream decoder",
    },
    FormatCapability {
        id: "ws-lzss-be-units",
        category: "compression",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "Big-endian unit-framed LZSS stream decoder with provenance",
    },
    FormatCapability {
        id: "ws-rle-77ee",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Escape-byte word and byte RLE stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-rle-pcm-00",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Zero-escaped byte RLE stream decoder and deterministic encoder for 8-bit PCM",
    },
    FormatCapability {
        id: "ws-counted-word-lz",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Word-aligned counted-command LZ stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-dpcm-6bit",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "6-bit packed signed DPCM audio stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lzss-11-5",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "2 KiB ring-buffer 11-bit position and 5-bit length LZSS stream decoder and encoder",
    },
    FormatCapability {
        id: "ws-planar-rle",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Bitplane-split 0xFFFF-terminated tile RLE stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lzss-token-be",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Big-endian token-counted bit-flag LZSS stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-nibble-lzss-ring8",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "4-bit nibble MSB-bitstream 256-entry ring-buffer LZSS stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-byte-lzss-ring8",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "8-bit byte MSB-bitstream 256-entry ring-buffer LZSS stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-zero-rle16",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "16-bit little-endian zero-run RLE stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lz-tile-7b",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "7-bit literal and back-reference 4bpp tile LZ stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-dpcm-4bit-nibble",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "4-bit nibble-delta DPCM audio stream decoder with hold RLE and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lz-rle-pat",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Byte-tagged literal, pattern-repeat, 10-bit LZ77, and RLE fill stream decoder and encoder",
    },
    FormatCapability {
        id: "ws-lzss-bandai-g",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Length-prefixed LSB-flag 11-bit distance single-plane and 2-plane LZSS stream decoder and encoder",
    },
    FormatCapability {
        id: "ws-rle16-bit15",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "16-bit little-endian word RLE (bit 15 repeat) stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-tile-xor-pack",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Tile-level bitmask and XOR-delta stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lzss-11-5-direct",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Direct-distance 11-bit offset and 5-bit length LZSS stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-huffman-static",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Static binary Huffman stream decoder and deterministic encoder with external tree table",
    },
    FormatCapability {
        id: "ws-rle-lz4-cmd",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "4-mode byte-command literal, RLE, and 8/16-bit LZ stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-word-rle-00",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "16-bit little-endian word RLE (bit 15 literal) stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-dpcm-4bit-sm",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "4-bit sign-magnitude scaled DPCM audio stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-wave-4bit-packed",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "4-bit packed unsigned wavetable-streamed PCM audio decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-tilemap-2bit",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "2-bit MSB-first delta/repeat/literal 16-bit tilemap stream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lzss-12-4-msb",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "MSB-first bitstream 12-bit ring position and 4-bit length LZSS decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-strided-rle",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "Plane-strided RLE stream decoder and deterministic encoder with per-plane zero terminators",
    },
    FormatCapability {
        id: "ws-dict16-rle",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "16-byte FIFO dictionary and repeat-last-byte MSB-first bitstream decoder and deterministic encoder",
    },
    FormatCapability {
        id: "ws-lz44",
        category: "compression",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 2,
        notes: "1-byte-token 4-bit distance and 4-bit length LSB-flag LZ77 decoder and deterministic encoder",
    },
    // Graphics (ws-gfx)
    FormatCapability {
        id: "ws-tile-2bpp",
        category: "gfx",
        capabilities: &["parse", "decode", "reproduce", "edit"],
        tier: 2,
        notes: "2bpp planar 8x8 tiles (monochrome and 4-color palettes)",
    },
    FormatCapability {
        id: "ws-tile-4bpp-planar",
        category: "gfx",
        capabilities: &["parse", "decode", "reproduce", "edit"],
        tier: 2,
        notes: "4bpp planar 8x8 WonderSwan Color tiles",
    },
    FormatCapability {
        id: "ws-tile-4bpp-packed",
        category: "gfx",
        capabilities: &["parse", "decode", "reproduce", "edit"],
        tier: 2,
        notes: "4bpp packed 8x8 WonderSwan Color tiles",
    },
    FormatCapability {
        id: "ws-mono-palettes",
        category: "gfx",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "Monochrome shade lookup tables and 8-palette register decoding",
    },
    FormatCapability {
        id: "ws-color-palettes",
        category: "gfx",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "12-bit BGR444 color palette tables (16 palettes x 16 colors)",
    },
    FormatCapability {
        id: "ws-screen-map",
        category: "gfx",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "32x32 16-bit background screen map entries",
    },
    FormatCapability {
        id: "ws-sprite-table",
        category: "gfx",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "32-bit OAM sprite table entries",
    },
    FormatCapability {
        id: "ws-display-snapshot",
        category: "gfx",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "64 KiB RAM + 256-byte I/O port display snapshot renderer (hardware and diagnostic profiles)",
    },
    // Text and fonts (ws-text)
    FormatCapability {
        id: "ws-character-map",
        category: "text",
        capabilities: &["parse", "decode", "reproduce", "edit"],
        tier: 2,
        notes: "Data-defined character maps (swankit-charmap/1) with control codes and shift pages",
    },
    FormatCapability {
        id: "ws-text-table",
        category: "text",
        capabilities: &["parse", "decode", "reproduce", "edit"],
        tier: 2,
        notes: "Definition-driven fixed, delimited, and pointer-table string extraction and re-encoding",
    },
    FormatCapability {
        id: "ws-font-strip",
        category: "text",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "Bitmap tile font atlas extraction from ROM definitions",
    },
    // Sound and music drivers (ws-sound)
    FormatCapability {
        id: "ws-sound.wave",
        category: "audio",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "4-bit packed hardware wave RAM banks and channel waveforms",
    },
    FormatCapability {
        id: "ws-sound.unsigned-pcm",
        category: "audio",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "8-bit unsigned hardware PCM sample streams and 24 kHz WAV preview rendering",
    },
    FormatCapability {
        id: "ws-sound.signed-pcm",
        category: "audio",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "8-bit signed PCM sample streams and WAV rendering",
    },
    FormatCapability {
        id: "ws-sound.byte-sequence",
        category: "audio",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "Byte-oriented sound driver sequence decoding and preview rendering",
    },
    FormatCapability {
        id: "ws-sound.cell-sequence",
        category: "audio",
        capabilities: &["parse", "decode"],
        tier: 2,
        notes: "Cell-oriented sound driver sequence decoding and WAV/event rendering",
    },
    FormatCapability {
        id: "ws-sound.sdk-sequence",
        category: "audio",
        capabilities: &["parse", "decode"],
        tier: 3,
        notes: "SDK sound driver bank discovery, sequence decoding, and hardware-matched WAV/event rendering",
    },
    // Patches (ws-patch)
    FormatCapability {
        id: "ws-patch-ips",
        category: "patch",
        capabilities: &["parse", "decode", "reproduce", "edit", "author"],
        tier: 2,
        notes: "IPS binary patch creation, validation, and policy-checked application with RLE and EOF truncation",
    },
    // Saves (ws-save, ws-saves)
    FormatCapability {
        id: "ws-save-image",
        category: "save",
        capabilities: &["parse", "decode", "edit"],
        tier: 2,
        notes: "Cartridge SRAM/EEPROM and internal EEPROM media geometry and checksum verification",
    },
    FormatCapability {
        id: "ws-defined-save",
        category: "save",
        capabilities: &["parse", "decode", "reproduce", "edit"],
        tier: 2,
        notes: "Declarative save definitions (swankit-save-def/1, swankit-save-def/2) for field decode, checked edits, and transactional repair",
    },
    // Mapped assets (ws-assets)
    FormatCapability {
        id: "ws-asset-map",
        category: "assets",
        capabilities: &["parse", "decode", "reproduce", "edit"],
        tier: 2,
        notes: "Per-ROM declarative asset maps for verification, extraction, checked replacement, relocation, and discovery",
    },
    // Serial link relay (ws-link)
    FormatCapability {
        id: "ws-link-relay",
        category: "link",
        capabilities: &["parse", "decode", "reproduce"],
        tier: 2,
        notes: "Unix serial byte-stream relay across TCP, stdio, exec, raw device, and WonderGate endpoints",
    },
    // WonderGate adapter and services (ws-wondergate, ws-wondergate-server)
    FormatCapability {
        id: "ws-wondergate",
        category: "wondergate",
        capabilities: &["parse", "decode", "reproduce", "author"],
        tier: 3,
        notes: "WonderGate adapter framing, excomm adapter, wire tap decoder, and local HTTP/BPL service emulation",
    },
];

static SAVE_ENGINES: &[&str] = &["swankit-save-def/1", "swankit-save-def/2"];
static PATCH_FORMATS: &[&str] = &["ips"];

pub fn capabilities_manifest() -> CapabilitiesManifest {
    CapabilitiesManifest {
        schema: CAPABILITIES_SCHEMA,
        tool: "swankit",
        version: env!("CARGO_PKG_VERSION"),
        formatkit_version: env!("SWANKIT_FORMATKIT_VERSION"),
        formats: FORMATS,
        save_engines: SAVE_ENGINES,
        patch_formats: PATCH_FORMATS,
    }
}

pub fn format_capabilities_table(manifest: &CapabilitiesManifest) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "{} {} (FormatKit {})",
        manifest.tool, manifest.version, manifest.formatkit_version
    ));
    lines.push(format!(
        "Save engines: {}",
        manifest.save_engines.join(", ")
    ));
    lines.push(format!(
        "Patch formats: {}",
        manifest.patch_formats.join(", ")
    ));
    lines.push(String::new());
    lines.push(format!(
        "{:<24} {:<12} {:<4} {:<36} {}",
        "ID", "CATEGORY", "TIER", "CAPABILITIES", "NOTES"
    ));
    lines.push(format!(
        "{:-<24} {:-<12} {:-<4} {:-<36} {:-<20}",
        "", "", "", "", ""
    ));
    for fmt in manifest.formats {
        lines.push(format!(
            "{:<24} {:<12} {:<4} {:<36} {}",
            fmt.id,
            fmt.category,
            fmt.tier,
            fmt.capabilities.join(","),
            fmt.notes
        ));
    }
    lines.join("\n")
}
