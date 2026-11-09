//! A fixed-register code-pattern lead, never a disassembler or caller analysis.
//! Instruction encodings: Intel 8086 Family User's Manual, instruction reference.
use formatkit_core::Reader;

pub(super) const LENGTH: usize = 73;

/// Fixed instruction roles for a bank/table-to-queue constructor. Only literal
/// table, segment and queue-pointer operands vary; no instruction is normalized.
/// Finding this byte sequence does not establish its instruction boundaries.
pub(super) fn evidence(src: &[u8]) -> Option<String> {
    const CORE: [u8; LENGTH] = [
        0xbf, 0, 0, 0xb8, 0, 0, 0x8e, 0xc0, 0x33, 0xc0, 0x8e, 0xd8, 0x8a, 0xc6, 0xd1, 0xe0, 0xd1,
        0xe0, 0x03, 0xf8, 0x26, 0x8b, 0x45, 0x02, 0xe6, 0xc3, 0x26, 0x8b, 0x3d, 0x50, 0xb8, 0,
        0x30, 0x8e, 0xc0, 0x32, 0xf6, 0xd1, 0xe2, 0x03, 0xfa, 0x26, 0x8b, 0x3d, 0x8b, 0x36, 0, 0,
        0x58, 0x89, 0x04, 0x8b, 0xc7, 0x89, 0x44, 0x02, 0x89, 0x4c, 0x04, 0x89, 0x5c, 0x06, 0x83,
        0xc6, 0x08, 0xc7, 0x04, 0, 0, 0x89, 0x36, 0, 0,
    ];
    let bytes = src.get(..LENGTH)?;
    for (at, (&actual, &expected)) in bytes.iter().zip(&CORE).enumerate() {
        if ![1, 2, 4, 5, 46, 47, 71, 72].contains(&at) && actual != expected {
            return None;
        }
    }
    let table = Reader::new(&bytes[1..3]).u16_le().ok()?;
    let segment = Reader::new(&bytes[4..6]).u16_le().ok()?;
    let queue = Reader::new(&bytes[46..48]).u16_le().ok()?;
    let queue_store = Reader::new(&bytes[71..73]).u16_le().ok()?;
    if queue != queue_store {
        return None;
    }
    Some(format!(
        "Fixed-role constructor opcode core: literal ES=0x{segment:04x}, table=0x{table:04x}, queue=0x{queue:04x}; DH*4 group lookup, OUT 0xc3 from AL of the loaded bank word, literal pointer-window ES=0x3000, DL*2 pointer lookup and 8-byte queue stride. The full bank word is retained by PUSH/POP AX. No caller selector, bank value, table extent or target data is resolved; this is a code-pattern lead only."
    ))
}
