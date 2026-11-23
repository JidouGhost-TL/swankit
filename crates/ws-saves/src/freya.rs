//! Read the observed bank-zero RAM volume described by the bank-three SoftFS directory.
use crate::value::bad;
use formatkit_core::{bytes_at, Endian, Result};
use formatkit_layout::{FixedRecords, U16Field, U32Field};

const BANK_SIZE: usize = 0x1_0000;
const MEDIA_SIZE: usize = 4 * BANK_SIZE;
const VOLUME_NAME: usize = 0x303b2;
const DRIVER_NAME: usize = 0x303c2;
const VOLUME_START: usize = 0x303da;
const VOLUME_LENGTH: usize = 0x303de;
const DIRECTORY_COUNT: usize = 0x303e2;
const DIRECTORY_START: usize = 0x306f2;
const DIRECTORY_WIDTH: usize = 0x40;
const BLOCK_WIDTH: usize = 0x80;

#[derive(Debug)]
pub(crate) struct File<'a> {
    pub directory_offset: usize,
    pub offset: usize,
    pub blocks: u16,
    pub bytes: &'a [u8],
}

pub(crate) fn read_file<'a>(image: &'a [u8], name: &[u8]) -> Result<File<'a>> {
    if image.len() != MEDIA_SIZE {
        return Err(bad("Freya SoftFS requires a 256 KiB cartridge SRAM image"));
    }
    let start = U32Field::new(VOLUME_START, Endian::Little).read(image)?;
    let length = U32Field::new(VOLUME_LENGTH, Endian::Little).read(image)?;
    let count = U16Field::new(DIRECTORY_COUNT, Endian::Little).read(image)?;
    if bytes_at(image, VOLUME_NAME, 5)? != b"ram0\0"
        || bytes_at(image, DRIVER_NAME, 8)? != b"@softfs\0"
        || start != 0
        || length as usize != BANK_SIZE
        || count != 64
    {
        return Err(bad("expected the four-bank Freya SoftFS RAM volume"));
    }

    let records = FixedRecords::<DIRECTORY_WIDTH>::new(DIRECTORY_START);
    let mut allocations = Vec::new();
    let mut found = None;
    for (index, record) in records.iter(image, usize::from(count))?.enumerate() {
        let blocks = U16Field::new(0x30, Endian::Little).read(record)?;
        if blocks == u16::MAX {
            continue;
        }
        let (entry_name, flags) = (
            record[..16]
                .split(|byte| *byte == 0)
                .next()
                .unwrap_or_default(),
            U16Field::new(0x32, Endian::Little).read(record)?,
        );
        if entry_name.is_empty() || flags != 6 {
            return Err(bad("unsupported Freya SoftFS directory entry"));
        }

        let offset = U32Field::new(0x28, Endian::Little).read(record)? as usize;
        let size = U32Field::new(0x2c, Endian::Little).read(record)? as usize;
        let allocation_bytes = usize::from(blocks) * BLOCK_WIDTH;
        if !offset.is_multiple_of(BLOCK_WIDTH) || size > allocation_bytes {
            return Err(bad("invalid Freya SoftFS file allocation"));
        }
        let Some(end) = offset
            .checked_add(allocation_bytes)
            .filter(|end| *end <= BANK_SIZE)
        else {
            return Err(bad("Freya SoftFS allocation crosses the data bank"));
        };
        if allocation_bytes != 0 {
            allocations.push((offset, end));
        }
        if entry_name == name {
            if found.is_some() {
                return Err(bad("duplicate Freya SoftFS filename"));
            }
            found = Some(File {
                directory_offset: DIRECTORY_START + index * DIRECTORY_WIDTH,
                offset,
                blocks,
                bytes: bytes_at(image, offset, size)?,
            });
        }
    }
    allocations.sort_unstable();
    if allocations.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(bad("overlapping Freya SoftFS file allocations"));
    }
    found.ok_or_else(|| bad("file absent from Freya SoftFS directory"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(image: &mut [u8], index: usize, name: &[u8], offset: u32, size: u32, blocks: u16) {
        let start = DIRECTORY_START + index * DIRECTORY_WIDTH;
        let entry = &mut image[start..start + DIRECTORY_WIDTH];
        entry.fill(0);
        entry[..name.len()].copy_from_slice(name);
        U32Field::new(0x28, Endian::Little)
            .write(entry, offset)
            .unwrap();
        U32Field::new(0x2c, Endian::Little)
            .write(entry, size)
            .unwrap();
        U16Field::new(0x30, Endian::Little)
            .write(entry, blocks)
            .unwrap();
        U16Field::new(0x32, Endian::Little).write(entry, 6).unwrap();
    }

    fn image() -> Vec<u8> {
        let mut image = vec![0; MEDIA_SIZE];
        image[VOLUME_NAME..VOLUME_NAME + 5].copy_from_slice(b"ram0\0");
        image[DRIVER_NAME..DRIVER_NAME + 8].copy_from_slice(b"@softfs\0");
        U32Field::new(VOLUME_LENGTH, Endian::Little)
            .write(&mut image, BANK_SIZE as u32)
            .unwrap();
        U16Field::new(DIRECTORY_COUNT, Endian::Little)
            .write(&mut image, 64)
            .unwrap();
        for index in 0..64 {
            record(&mut image, index, b"", 0, 0, u16::MAX);
        }
        image
    }

    #[test]
    fn reads_relocated_file_after_free_slot_with_stale_name() {
        let mut image = image();
        record(&mut image, 0, b"first.dat", 0, 3, u16::MAX);
        record(&mut image, 3, b"first.dat", 0x400, 3, 1);
        image[0x400..0x403].copy_from_slice(b"abc");
        let file = read_file(&image, b"first.dat").unwrap();
        assert_eq!(file.directory_offset, 0x307b2);
        assert_eq!(file.offset, 0x400);
        assert_eq!(file.blocks, 1);
        assert_eq!(file.bytes, b"abc");
    }

    #[test]
    fn refuses_duplicate_overlap_and_invalid_extent() {
        let mut image = image();
        record(&mut image, 0, b"first.dat", 0x400, 3, 2);
        record(&mut image, 1, b"first.dat", 0, 3, 1);
        assert!(read_file(&image, b"first.dat")
            .unwrap_err()
            .to_string()
            .contains("duplicate"));

        record(&mut image, 1, b"second.dat", 0x480, 3, 1);
        assert!(read_file(&image, b"first.dat")
            .unwrap_err()
            .to_string()
            .contains("overlapping"));

        record(&mut image, 1, b"second.dat", 0xff80, 3, 2);
        assert!(read_file(&image, b"first.dat")
            .unwrap_err()
            .to_string()
            .contains("data bank"));

        record(&mut image, 1, b"second.dat", 0x500, 129, 1);
        assert!(read_file(&image, b"first.dat")
            .unwrap_err()
            .to_string()
            .contains("invalid"));
    }
}
