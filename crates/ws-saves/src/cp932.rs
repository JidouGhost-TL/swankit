//! Python CP932 single-byte extensions and error-consumption policy.
use crate::value::bad;
use formatkit_core::Result;

pub(crate) fn decode(bytes: &[u8], errors: &str) -> Result<String> {
    let mut text = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let lead = bytes[index];
        let single = match lead {
            0..=0x80 => Some(u32::from(lead)),
            0xa0 => Some(0xf8f0),
            0xa1..=0xdf => Some(0xff61 + u32::from(lead - 0xa1)),
            0xfd..=0xff => Some(0xf8f1 + u32::from(lead - 0xfd)),
            _ => None,
        };
        if let Some(code) = single {
            text.push(char::from_u32(code).unwrap());
            index += 1;
            continue;
        }
        if index + 1 < bytes.len() {
            let (decoded, error) =
                encoding_rs::SHIFT_JIS.decode_without_bom_handling(&bytes[index..index + 2]);
            if !error {
                text.push_str(&decoded);
                index += 2;
                continue;
            }
        }
        // Python retries the following byte after an invalid/unmapped pair.
        match errors {
            "strict" => return Err(bad("invalid cp932 encoded text")),
            "ignore" => {}
            "replace" => text.push('\u{fffd}'),
            _ => return Err(bad("unsupported text error policy")),
        }
        index += 1;
    }
    Ok(text)
}
