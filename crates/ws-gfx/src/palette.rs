//! Palette rules: <https://ws.nesdev.org/wiki/Display/IO_Ports> and
//! <https://ws.nesdev.org/wiki/Display/Palette>.
use crate::invalid;
use formatkit_core::{Reader, Result};

/// Colour zero is transparent for palettes 4-7/12-15 in 2bpp,
/// and in every palette in 4bpp. Other indices are opaque.
pub const fn transparent(palette: u8, index: u8, format: crate::TileFormat) -> bool {
    index == 0 && (format.colors() == 16 || palette & 4 != 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonoPalettes {
    shades: [[u8; 4]; 8],
    palettes: [[[u8; 4]; 4]; 16],
}

impl MonoPalettes {
    /// Parse the 8 four-bit luminance levels and 16 four-entry shade LUTs.
    /// Larger levels are darker; unused palette nibble bits are ignored.
    pub fn from_ports(ports: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(ports);
        reader.skip(0x1c)?;
        let levels = reader.bytes(4)?;
        let selections = reader.bytes(32)?;
        let mut shades = [[0; 4]; 8];
        for (i, shade) in shades.iter_mut().enumerate() {
            let g = 255 - ((levels[i / 2] >> (4 * (i % 2))) & 15) * 17;
            *shade = [g, g, g, 255];
        }
        let mut palettes = [[[0; 4]; 4]; 16];
        for (p, palette) in palettes.iter_mut().enumerate() {
            for (i, color) in palette.iter_mut().enumerate() {
                let shade = (selections[p * 2 + i / 2] >> (4 * (i % 2))) & 7;
                *color = shades[shade as usize];
            }
        }
        Ok(Self { shades, palettes })
    }
    pub fn shade(&self, index: u8) -> Result<[u8; 4]> {
        self.shades
            .get(index as usize)
            .copied()
            .ok_or_else(|| invalid("mono shade", index as usize))
    }
    pub fn palette(&self, index: u8) -> Result<&[[u8; 4]; 4]> {
        self.palettes
            .get(index as usize)
            .ok_or_else(|| invalid("mono palette", index as usize))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorPalettes {
    palettes: [[[u8; 4]; 16]; 16],
}

impl ColorPalettes {
    /// Parse exactly the 512-byte little-endian RGB444 palette region.
    /// Reserved high bits are ignored; each channel expands by nibble replication.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        let mut palettes = [[[0; 4]; 16]; 16];
        for palette in &mut palettes {
            for color in palette {
                let word = reader.u16_le()?;
                *color = [
                    ((word >> 8) & 15) as u8 * 17,
                    ((word >> 4) & 15) as u8 * 17,
                    (word & 15) as u8 * 17,
                    255,
                ];
            }
        }
        if reader.remaining() != 0 {
            return Err(formatkit_core::Error::Trailing(reader.remaining()));
        }
        Ok(Self { palettes })
    }
    pub fn palette(&self, index: u8) -> Result<&[[u8; 4]; 16]> {
        self.palettes
            .get(index as usize)
            .ok_or_else(|| invalid("colour palette", index as usize))
    }
}
