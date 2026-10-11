//! Composed WonderSwan catalog and typed operations; file I/O belongs at edges.
#![forbid(unsafe_code)]
pub mod catalog;
pub mod extract;
mod operations;
#[cfg(feature = "saves")]
pub mod save;
use formatkit_catalog::DetectionContext;
use formatkit_core::{Error, Result};
use formatkit_runtime::ModuleCatalog;
use serde::Serialize;
pub use ws_compress as compress;
pub use ws_gfx as gfx;
pub use ws_patch as patch;
pub use ws_rom as rom;
#[cfg(feature = "saves")]
pub use ws_save as save_image;
#[cfg(feature = "saves")]
pub use ws_saves as saves;
pub use ws_sound as sound;
pub use ws_text as text;

pub struct Toolkit {
    catalog: ModuleCatalog,
}
impl Toolkit {
    pub fn new() -> Result<Self> {
        Ok(Self {
            catalog: catalog::compose().map_err(|e| Error::Malformed(e.to_string()))?,
        })
    }
    pub fn catalog(&self) -> &ModuleCatalog {
        &self.catalog
    }
    pub fn detect(&self, bytes: &[u8], extension: Option<&str>) -> Detection {
        let result = self.catalog.formats().resolve(DetectionContext {
            bytes,
            extension,
            ..DetectionContext::default()
        });
        Detection {
            confidence: result
                .selected
                .map(|c| c.descriptor.capabilities.confidence.label().to_owned()),
            #[cfg(feature = "saves")]
            save_media: if result
                .selected
                .is_some_and(|c| c.descriptor.id == save_image::catalog::SAVE_IMAGE)
            {
                save_image::catalog::possible_media(bytes.len())
                    .into_iter()
                    .map(|m| format!("{m:?}"))
                    .collect()
            } else {
                Vec::new()
            },
            #[cfg(feature = "saves")]
            save_definitions: Vec::new(),
            selected: result.selected.map(|c| c.descriptor.id.as_str().to_owned()),
            candidates: result
                .contenders
                .iter()
                .map(|c| c.descriptor.id.as_str().to_owned())
                .collect(),
        }
    }
    pub fn info(&self, bytes: &[u8]) -> Result<RomInfo> {
        if self.detect(bytes, None).selected.as_deref() != Some(rom::catalog::ROM_ID.as_str()) {
            return Err(Error::Malformed(
                "input is not a recognized WonderSwan cartridge image".into(),
            ));
        }
        let f = rom::RomFacts::parse(bytes)?;
        let h = f.header;
        Ok(RomInfo {
            format: "ws-rom",
            size_bytes: bytes.len(),
            size_pow2: f.geometry.is_power_of_two(),
            publisher_id: h.publisher_id,
            publisher: rom::publisher_name(h.publisher_id),
            color: h.color(),
            game_id: h.game_id,
            version: h.version,
            rom_size_code: h.rom_size_code,
            declared_rom_bytes: h.declared_rom_size(),
            declared_size_matches: f.declared_size_matches(),
            save_code: h.save_code,
            save: format!("{:?}", h.save()),
            // WSdev and the Java hardware port map code 01 to observed 32 KiB.
            // h.save() retains the separate nominal 64 Kbit declaration.
            save_bytes: if h.save_code == 1 {
                Some(32768)
            } else {
                h.save().bytes()
            },
            flags: h.flags,
            orientation: match h.orientation() {
                rom::Orientation::Horizontal => "horizontal",
                rom::Orientation::Vertical => "vertical",
            },
            bus_width: h.bus_width(),
            rom_speed_cycles: h.rom_speed_cycles(),
            mapper_code: h.mapper_code,
            mapper: format!("{:?}", h.mapper()),
            maintenance: h.maintenance,
            reset_segment: h.reset_segment,
            reset_offset: h.reset_offset,
            reset_linear: h.reset_linear(),
            checksum_footer: h.checksum,
            checksum_computed: f.checksum_computed,
            checksum_ok: f.checksum_ok(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    pub confidence: Option<String>,
    #[cfg(feature = "saves")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub save_media: Vec<String>,
    #[cfg(feature = "saves")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub save_definitions: Vec<save::DefinitionCandidate>,
    pub selected: Option<String>,
    pub candidates: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct RomInfo {
    pub format: &'static str,
    pub size_bytes: usize,
    pub size_pow2: bool,
    pub publisher_id: u8,
    pub publisher: Option<&'static str>,
    pub color: bool,
    pub game_id: u8,
    pub version: u8,
    pub rom_size_code: u8,
    pub declared_rom_bytes: Option<usize>,
    pub declared_size_matches: bool,
    pub save_code: u8,
    pub save: String,
    pub save_bytes: Option<u32>,
    pub flags: u8,
    pub orientation: &'static str,
    pub bus_width: u8,
    pub rom_speed_cycles: u8,
    pub mapper_code: u8,
    pub mapper: String,
    pub maintenance: u8,
    pub reset_segment: u16,
    pub reset_offset: u16,
    pub reset_linear: u32,
    pub checksum_footer: u16,
    pub checksum_computed: u16,
    pub checksum_ok: bool,
}
