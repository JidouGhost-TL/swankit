//! Strict JIS X 0208 Shift-JIS decoding matching Python's `shift_jis` codec.
//! Excludes Windows-31J vendor extensions and preserves standard JIS mappings.
use crate::value::bad;
use formatkit_core::text::{
    decode_jis0208_pair, decode_shift_jis_jis0208, decode_shift_jis_jis0208_lossy,
};
use formatkit_core::Result;

pub(crate) fn decode(bytes: &[u8], errors: &str) -> Result<String> {
    match errors {
        "strict" => {
            decode_shift_jis_jis0208(bytes).map_err(|_| bad("invalid shift_jis encoded text"))
        }
        "replace" => Ok(decode_shift_jis_jis0208_lossy(bytes)),
        "ignore" => {
            let mut text = String::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                let lead = bytes[i];
                if lead < 0x80 {
                    text.push(char::from(lead));
                    i += 1;
                    continue;
                }
                if (0xa1..=0xdf).contains(&lead) {
                    text.push(char::from_u32(0xff61 + u32::from(lead - 0xa1)).unwrap());
                    i += 1;
                    continue;
                }
                if let Some(&trail) = bytes.get(i + 1) {
                    if let Some(c) = decode_jis0208_pair(lead, trail) {
                        text.push(c);
                        i += 2;
                        continue;
                    }
                }
                i += 1;
            }
            Ok(text)
        }
        _ => Err(bad("unsupported text error policy")),
    }
}
