//! Experimental Java interop boundary. Format grammars remain in the owner crates.
//! Hardware sources: ws-rom/header and ws-gfx/tile/palette module documentation.
use formatkit_core::{Error, Reader, Result};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub mod protocol;

pub const MAX_INPUT: usize = ws_rom::MAX_ROM_SIZE;
pub const MAX_TILES: usize = 1024;
pub const MAX_OUTPUT: usize = MAX_TILES * 256;
pub const HEADER_BYTES: usize = 18 * 4;

/// Allocate a zeroed buffer in Wasm linear memory. Returns null for invalid lengths.
/// The Java adapter owns the buffer and releases it with the same length.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn swankit_alloc_v1(length: u32) -> *mut u8 {
    if length == 0 || length as usize > MAX_INPUT {
        return std::ptr::null_mut();
    }
    Box::into_raw(vec![0u8; length as usize].into_boxed_slice()).cast::<u8>()
}

/// Release a buffer obtained from `swankit_alloc_v1`.
///
/// # Safety
/// The pointer and length must identify a live allocation from that function,
/// and the buffer must not be used again after this call.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn swankit_free_v1(pointer: *mut u8, length: u32) {
    if !pointer.is_null() && length != 0 && length as usize <= MAX_INPUT {
        // SAFETY: the caller transfers its original boxed slice back to this allocator.
        unsafe {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                pointer,
                length as usize,
            )))
        };
    }
}

/// Operation 1: whole ROM to 18 little-endian u32 facts (see README).
/// Operations 2/3: RGB444 palette region (512 bytes), then planar/packed tiles.
/// Palette zero is used, with opaque alpha matching the Java asset image API.
pub fn decode(operation: u32, input: &[u8]) -> Result<Vec<u8>> {
    if input.len() > MAX_INPUT {
        return Err(Error::ResourceLimit {
            resource: "interop input",
            requested: input.len() as u64,
            limit: MAX_INPUT as u64,
        });
    }
    match operation {
        1 => {
            let facts = ws_rom::RomFacts::parse(input)?;
            let h = facts.header;
            let words = [
                u32::from(h.reset_offset),
                u32::from(h.reset_segment),
                u32::from(h.maintenance),
                u32::from(h.publisher_id),
                u32::from(h.color()),
                u32::from(h.game_id),
                u32::from(h.version),
                u32::from(h.rom_size_code),
                u32::from(h.save_code),
                u32::from(h.flags),
                u32::from(h.mapper_code),
                u32::from(h.mapper_code == 1),
                u32::from(h.checksum),
                u32::from(facts.checksum_computed),
                u32::from(facts.checksum_ok()),
                h.declared_rom_size().map_or(u32::MAX, |n| n as u32),
                h.reset_linear(),
                input.len() as u32,
            ];
            Ok(words.into_iter().flat_map(u32::to_le_bytes).collect())
        }
        2 | 3 => {
            let mut reader = Reader::new(input);
            let palettes = ws_gfx::ColorPalettes::parse(reader.bytes(512)?)?;
            let tile_bytes = reader.bytes(reader.remaining())?;
            if tile_bytes.is_empty()
                || !tile_bytes.len().is_multiple_of(32)
                || tile_bytes.len() / 32 > MAX_TILES
            {
                return Err(Error::Malformed(
                    "interop requires 1..=1024 complete 4bpp tiles".into(),
                ));
            }
            let format = if operation == 2 {
                ws_gfx::TileFormat::Bpp4Planar
            } else {
                ws_gfx::TileFormat::Bpp4Packed
            };
            let palette = palettes.palette(0)?;
            let mut output = Vec::with_capacity(tile_bytes.len() / 32 * 256);
            for offset in (0..tile_bytes.len()).step_by(32) {
                let indices = ws_gfx::decode_tile(tile_bytes, offset, format)?;
                output.extend_from_slice(&ws_gfx::render_tile(&indices, palette)?.rgba);
            }
            Ok(output)
        }
        _ => Err(Error::Malformed("unknown interop operation".into())),
    }
}

/// ABI version; callers must check before decoding.
#[no_mangle]
pub extern "C" fn swankit_abi_version() -> u32 {
    1
}

/// Return bytes written, or -1 (arguments), -2 (parse), -3 (capacity), -4 (panic).
/// Output is left untouched on error. Neither pointer is retained.
///
/// # Safety
/// `input` must address `input_len` readable bytes, and `output` must address
/// `output_capacity` writable bytes for the duration of the call. Both pointers
/// must be nonnull. No other thread may mutate these regions during the call.
/// Pointer validity cannot be checked by this ABI; native callers own that duty.
#[no_mangle]
pub unsafe extern "C" fn swankit_decode_v1(
    operation: u32,
    input: *const u8,
    input_len: u32,
    output: *mut u8,
    output_capacity: u32,
) -> i32 {
    if input.is_null()
        || output.is_null()
        || input_len as usize > MAX_INPUT
        || output_capacity as usize > MAX_OUTPUT
    {
        return -1;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: callers supply live regions under the documented contract; lengths are bounded above.
        let input = unsafe { std::slice::from_raw_parts(input, input_len as usize) };
        match decode(operation, input) {
            Err(_) => -2,
            Ok(bytes) if bytes.len() > output_capacity as usize => -3,
            Ok(bytes) => {
                // SAFETY: the owned result cannot overlap caller memory, and capacity was checked.
                unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
                bytes.len() as i32
            }
        }
    }))
    .unwrap_or(-4)
}
