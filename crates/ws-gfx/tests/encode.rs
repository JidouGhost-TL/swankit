//! The tile encoder is the exact inverse of the decoder for every format.
//! <https://ws.nesdev.org/wiki/Display/Tile_Data>
use ws_gfx::{decode_tile, encode_tile, TileFormat};

#[test]
fn tile_encoder_inverts_the_hardware_decoder() {
    for format in [
        TileFormat::Bpp2,
        TileFormat::Bpp4Planar,
        TileFormat::Bpp4Packed,
    ] {
        let mut indices = [0u8; 64];
        for (i, v) in indices.iter_mut().enumerate() {
            *v = ((i * 7 + i / 8) % format.colors()) as u8;
        }
        let bytes = encode_tile(&indices, format);
        assert_eq!(bytes.len(), format.bytes_per_tile());
        assert_eq!(decode_tile(&bytes, 0, format).unwrap(), indices);
        // And bytes -> indices -> bytes for an arbitrary stored tile.
        let stored: Vec<u8> = (0..format.bytes_per_tile() as u8)
            .map(|b| b.wrapping_mul(37) ^ 0x5a)
            .collect();
        let decoded = decode_tile(&stored, 0, format).unwrap();
        assert_eq!(encode_tile(&decoded, format), stored);
    }
}
