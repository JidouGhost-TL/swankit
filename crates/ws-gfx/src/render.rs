//! Maps, sprites and composition: WSdev Display, Screen, Sprites and IO Ports.
use crate::{image, invalid, transparent, ColorPalettes, MonoPalettes, TileFormat};
use formatkit_core::{Endian, Error, Image, Reader, Result};
use formatkit_layout::U16Field;

pub const WIDTH: u32 = 224;
pub const HEIGHT: u32 = 144;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapEntry {
    pub tile: u16,
    pub palette: u8,
    pub bank: bool,
    pub h_flip: bool,
    pub v_flip: bool,
}

impl MapEntry {
    /// Bits 0-8 tile, 9-12 palette, 13 bank (colour only), 14/15 H/V flip.
    pub const fn from_word(word: u16, color: bool) -> Self {
        Self {
            tile: (word & 0x1ff) | if color { (word & 0x2000) >> 4 } else { 0 },
            palette: ((word >> 9) & 15) as u8,
            bank: word & 0x2000 != 0,
            h_flip: word & 0x4000 != 0,
            v_flip: word & 0x8000 != 0,
        }
    }
    /// Parse a screen's little-endian entries. At most one 32x32 screen.
    pub fn parse(bytes: &[u8], color: bool) -> Result<Vec<Self>> {
        if !bytes.len().is_multiple_of(2) || bytes.len() > 2048 {
            return Err(invalid("screen map bytes", bytes.len()));
        }
        let mut reader = Reader::new(bytes);
        let mut entries = Vec::with_capacity(bytes.len() / 2);
        while reader.remaining() != 0 {
            entries.push(Self::from_word(reader.u16_le()?, color));
        }
        Ok(entries)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sprite {
    pub tile: u16,
    pub palette: u8,
    pub x: u8,
    pub y: u8,
    pub h_flip: bool,
    pub v_flip: bool,
    pub high_priority: bool,
    pub inside_window: bool,
}

impl Sprite {
    /// Four-byte record: little-endian attributes, Y, X. Sprites use bank 0.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let attr = r.u16_le()?;
        let y = r.u8()?;
        let x = r.u8()?;
        if r.remaining() != 0 {
            return Err(Error::Trailing(r.remaining()));
        }
        Ok(Self {
            tile: attr & 0x1ff,
            palette: 8 + ((attr >> 9) & 7) as u8,
            x,
            y,
            h_flip: attr & 0x4000 != 0,
            v_flip: attr & 0x8000 != 0,
            high_priority: attr & 0x2000 != 0,
            inside_window: attr & 0x1000 == 0,
        })
    }
}

/// Explicitly select the documented snapshot rules or the extension's diagnostic behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderProfile {
    /// Documented banks, masks, sprite ordering and 32 sprites per scanline.
    #[default]
    Hardware,
    /// Reproduce WSRender/WSAssets: 2bpp bank alias, unmasked mono map/table
    /// address bits, priority-group ordering and no sprite limit. For parity
    /// with existing asset tools.
    JavaDiagnostic,
}

/// Borrowed, validated 64 KiB RAM and 256-byte port snapshot. Mono RAM dumps
/// must be zero-extended by the caller; no implicit memory mirroring is inferred.
pub struct Snapshot<'a> {
    ram: &'a [u8],
    ports: &'a [u8; 256],
    color_hardware: bool,
    line_ports: Option<&'a [[u8; 256]]>,
}

impl<'a> Snapshot<'a> {
    pub fn new(ram: &'a [u8], ports: &'a [u8], color_hardware: bool) -> Result<Self> {
        let mut r = Reader::new(ram);
        let ram = r.bytes(0x10000)?;
        if r.remaining() != 0 {
            return Err(Error::Trailing(r.remaining()));
        }
        let mut r = Reader::new(ports);
        let row = r.bytes(256)?;
        if r.remaining() != 0 {
            return Err(Error::Trailing(r.remaining()));
        }
        let ports = row
            .try_into()
            .map_err(|_| invalid("display ports", row.len()))?;
        Ok(Self {
            ram,
            ports,
            color_hardware,
            line_ports: None,
        })
    }

    /// Optional port row for every visible line. RAM is still from one instant;
    /// palette RAM and tile changes within the frame cannot be reconstructed.
    pub fn with_line_ports(mut self, rows: &'a [[u8; 256]]) -> Result<Self> {
        if rows.len() != HEIGHT as usize {
            return Err(invalid("visible port rows", rows.len()));
        }
        self.line_ports = Some(rows);
        Ok(self)
    }

    pub fn tile_format(&self) -> TileFormat {
        mode(self.ports, self.color_hardware).1
    }

    pub fn map(&self, nibble: u8, profile: RenderProfile) -> Result<Vec<MapEntry>> {
        if nibble > 15 {
            return Err(invalid("screen address nibble", nibble as usize));
        }
        let base = self.map_base(nibble, profile);
        MapEntry::parse(
            &self.ram[base..base + 2048],
            mode(self.ports, self.color_hardware).0,
        )
    }

    pub fn sprites(&self, profile: RenderProfile) -> Result<Vec<Sprite>> {
        let mask = if profile == RenderProfile::Hardware && !self.color_hardware {
            0x1f
        } else {
            0x3f
        };
        let base = (self.ports[4] & mask) as usize * 0x200;
        let first = (self.ports[5] & 127) as usize;
        let end = (first + self.ports[6] as usize).min(128);
        (first..end)
            .map(|s| Sprite::parse(&self.ram[base + s * 4..base + s * 4 + 4]))
            .collect()
    }

    /// Render the LCD area in native orientation, with opaque RGBA output.
    pub fn render(&self, profile: RenderProfile) -> Result<Image> {
        let colors = ColorPalettes::parse(&self.ram[0xfe00..])?;
        let mut out = image(
            WIDTH,
            HEIGHT,
            WIDTH as usize * HEIGHT as usize * 4,
            [0, 0, 0, 255],
        )?;
        for y in 0..HEIGHT as usize {
            let ports = self.line_ports.map_or(self.ports, |rows| &rows[y]);
            let c = Context::new(self.ram, ports, self.color_hardware, profile, &colors)?;
            let sprites = c.sprites_for_line(y)?;
            for x in 0..WIDTH as usize {
                let mut pixel = c.background()?;
                let hardware_sprite = if profile == RenderProfile::Hardware {
                    c.sprite_pixel(&sprites, x, y, None)?
                } else {
                    None
                };
                if ports[0] & 1 != 0 {
                    if let Some(p) =
                        c.screen_pixel(ports[7] & 15, ports[0x10], ports[0x11], x, y)?
                    {
                        pixel = p;
                    }
                }
                if ports[0] & 4 != 0 {
                    let sprite = if profile == RenderProfile::Hardware {
                        hardware_sprite.filter(|s| !s.1)
                    } else {
                        c.sprite_pixel(&sprites, x, y, Some(false))?
                    };
                    if let Some((p, _)) = sprite {
                        pixel = p;
                    }
                }
                if ports[0] & 2 != 0
                    && (ports[0] & 0x20 == 0 || inside(ports, 8, x, y) == (ports[0] & 0x10 == 0))
                {
                    if let Some(p) =
                        c.screen_pixel(ports[7] >> 4, ports[0x12], ports[0x13], x, y)?
                    {
                        pixel = p;
                    }
                }
                if ports[0] & 4 != 0 {
                    let sprite = if profile == RenderProfile::Hardware {
                        hardware_sprite.filter(|s| s.1)
                    } else {
                        c.sprite_pixel(&sprites, x, y, Some(true))?
                    };
                    if let Some((p, _)) = sprite {
                        pixel = p;
                    }
                }
                let offset = (y * WIDTH as usize + x) * 4;
                out.rgba[offset..offset + 4].copy_from_slice(&pixel);
            }
        }
        Ok(out)
    }

    /// Render a full 256x256 screen without scroll. Transparent pixels use
    /// the caller's preview colour, or [0; 4] for an alpha-bearing layer.
    pub fn render_layer(
        &self,
        nibble: u8,
        profile: RenderProfile,
        clear: [u8; 4],
    ) -> Result<Image> {
        let entries = self.map(nibble, profile)?;
        self.render_map(&entries, 32, profile, clear, 256 * 256 * 4)
    }

    /// Render decoded map entries through this snapshot's tile and palette banks.
    pub fn render_map(
        &self,
        entries: &[MapEntry],
        columns: u32,
        profile: RenderProfile,
        clear: [u8; 4],
        max_bytes: usize,
    ) -> Result<Image> {
        if entries.is_empty() || entries.len() > 1024 || columns == 0 {
            return Err(invalid("map dimensions", entries.len()));
        }
        for e in entries {
            if e.tile >= 1024 || e.palette >= 16 {
                return Err(invalid("map tile or palette", e.tile as usize));
            }
        }
        let rows = entries.len().div_ceil(columns as usize) as u32;
        let width = columns
            .checked_mul(8)
            .ok_or_else(|| invalid("map width", columns as usize))?;
        let colors = ColorPalettes::parse(&self.ram[0xfe00..])?;
        let c = Context::new(self.ram, self.ports, self.color_hardware, profile, &colors)?;
        let mut out = image(width, rows * 8, max_bytes, clear)?;
        for (i, e) in entries.iter().enumerate() {
            for y in 0..8 {
                for x in 0..8 {
                    let pixel = c.entry_pixel(*e, x, y)?.unwrap_or(clear);
                    let o = ((i / columns as usize * 8 + y) * width as usize
                        + i % columns as usize * 8
                        + x)
                        * 4;
                    out.rgba[o..o + 4].copy_from_slice(&pixel);
                }
            }
        }
        Ok(out)
    }

    pub fn render_sprite(&self, sprite: Sprite, profile: RenderProfile) -> Result<Image> {
        if sprite.tile >= 512 || !(8..16).contains(&sprite.palette) {
            return Err(invalid("sprite tile or palette", sprite.tile as usize));
        }
        let colors = ColorPalettes::parse(&self.ram[0xfe00..])?;
        let c = Context::new(self.ram, self.ports, self.color_hardware, profile, &colors)?;
        let e = MapEntry {
            tile: sprite.tile,
            palette: sprite.palette,
            bank: false,
            h_flip: sprite.h_flip,
            v_flip: sprite.v_flip,
        };
        let mut out = image(8, 8, 256, [0; 4])?;
        for y in 0..8 {
            for x in 0..8 {
                let pixel = c.entry_pixel(e, x, y)?.unwrap_or([0; 4]);
                let o = (y * 8 + x) * 4;
                out.rgba[o..o + 4].copy_from_slice(&pixel);
            }
        }
        Ok(out)
    }

    fn map_base(&self, nibble: u8, profile: RenderProfile) -> usize {
        let mask = if profile == RenderProfile::Hardware && !self.color_hardware {
            7
        } else {
            15
        };
        (nibble & mask) as usize * 0x800
    }
}

fn mode(p: &[u8; 256], hardware: bool) -> (bool, TileFormat) {
    let color = hardware && p[0x60] & 0x80 != 0;
    let format = if color && p[0x60] & 0x40 != 0 {
        if p[0x60] & 0x20 != 0 {
            TileFormat::Bpp4Packed
        } else {
            TileFormat::Bpp4Planar
        }
    } else {
        TileFormat::Bpp2
    };
    (color, format)
}

fn inside(p: &[u8; 256], base: usize, x: usize, y: usize) -> bool {
    x >= p[base] as usize
        && x <= p[base + 2] as usize
        && y >= p[base + 1] as usize
        && y <= p[base + 3] as usize
}

struct Context<'a> {
    ram: &'a [u8],
    p: &'a [u8; 256],
    hardware: bool,
    profile: RenderProfile,
    color: bool,
    format: TileFormat,
    mono: MonoPalettes,
    colors: &'a ColorPalettes,
}

impl<'a> Context<'a> {
    fn new(
        ram: &'a [u8],
        p: &'a [u8; 256],
        hardware: bool,
        profile: RenderProfile,
        colors: &'a ColorPalettes,
    ) -> Result<Self> {
        let (color, format) = mode(p, hardware);
        Ok(Self {
            ram,
            p,
            hardware,
            profile,
            color,
            format,
            mono: MonoPalettes::from_ports(p)?,
            colors,
        })
    }
    fn colour(&self, palette: u8, index: u8) -> Result<[u8; 4]> {
        if self.color {
            Ok(self.colors.palette(palette)?[index as usize])
        } else {
            Ok(self.mono.palette(palette)?[index as usize])
        }
    }
    fn background(&self) -> Result<[u8; 4]> {
        if self.color {
            self.colour(self.p[1] >> 4, self.p[1] & 15)
        } else {
            self.mono.shade(self.p[1] & 7)
        }
    }
    fn tile_pixel(&self, tile: u16, x: usize, y: usize) -> u8 {
        let tile = if self.format == TileFormat::Bpp2
            && (!self.color || self.profile == RenderProfile::JavaDiagnostic)
        {
            tile & 511
        } else {
            tile
        } as usize;
        let base = if self.format == TileFormat::Bpp2 {
            0x2000
        } else {
            0x4000
        };
        let planes = if self.format == TileFormat::Bpp2 {
            2
        } else {
            4
        };
        let o = base + tile * self.format.bytes_per_tile() + y * planes;
        if self.format == TileFormat::Bpp4Packed {
            (self.ram[o + x / 2] >> (4 * (1 - x % 2))) & 15
        } else {
            (0..planes).fold(0, |v, p| v | (((self.ram[o + p] >> (7 - x)) & 1) << p))
        }
    }
    fn entry_pixel(&self, e: MapEntry, x: usize, y: usize) -> Result<Option<[u8; 4]>> {
        let x = if e.h_flip { 7 - x } else { x };
        let y = if e.v_flip { 7 - y } else { y };
        let i = self.tile_pixel(e.tile, x, y);
        if transparent(e.palette, i, self.format) {
            Ok(None)
        } else {
            self.colour(e.palette, i).map(Some)
        }
    }
    fn screen_pixel(
        &self,
        nibble: u8,
        sx: u8,
        sy: u8,
        x: usize,
        y: usize,
    ) -> Result<Option<[u8; 4]>> {
        let mask = if self.profile == RenderProfile::Hardware && !self.hardware {
            7
        } else {
            15
        };
        let x = (x + sx as usize) & 255;
        let y = (y + sy as usize) & 255;
        let o = (nibble & mask) as usize * 0x800 + ((y / 8) * 32 + x / 8) * 2;
        let word = U16Field::new(o, Endian::Little).read(self.ram)?;
        self.entry_pixel(MapEntry::from_word(word, self.color), x % 8, y % 8)
    }
    fn sprites_for_line(&self, y: usize) -> Result<Vec<Sprite>> {
        let mask = if self.profile == RenderProfile::Hardware && !self.hardware {
            0x1f
        } else {
            0x3f
        };
        let base = (self.p[4] & mask) as usize * 0x200;
        let first = (self.p[5] & 127) as usize;
        let end = (first + self.p[6] as usize).min(128);
        let mut out = Vec::with_capacity(end - first);
        if self.p[0] & 4 == 0 {
            return Ok(out);
        }
        for i in first..end {
            let s = Sprite::parse(&self.ram[base + i * 4..base + i * 4 + 4])?;
            if (y as u8).wrapping_sub(s.y) < 8 {
                out.push(s);
                if self.profile == RenderProfile::Hardware && out.len() == 32 {
                    break;
                }
            }
        }
        Ok(out)
    }
    fn sprite_pixel(
        &self,
        sprites: &[Sprite],
        x: usize,
        y: usize,
        high: Option<bool>,
    ) -> Result<Option<([u8; 4], bool)>> {
        for s in sprites {
            if high.is_some_and(|high| s.high_priority != high) {
                continue;
            }
            let dx = (x as u8).wrapping_sub(s.x) as usize;
            let dy = (y as u8).wrapping_sub(s.y) as usize;
            if dx >= 8 || dy >= 8 {
                continue;
            }
            if self.p[0] & 8 != 0 && inside(self.p, 12, x, y) != s.inside_window {
                continue;
            }
            let e = MapEntry {
                tile: s.tile,
                palette: s.palette,
                bank: false,
                h_flip: s.h_flip,
                v_flip: s.v_flip,
            };
            if let Some(pixel) = self.entry_pixel(e, dx, dy)? {
                return Ok(Some((pixel, s.high_priority)));
            }
        }
        Ok(None)
    }
}
