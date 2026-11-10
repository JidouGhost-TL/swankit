//! Mode-prefixed LZ stream with 16-bit LSB-first flags and 11-bit distances.
//! Zero flag bits are literals; one bits carry a little-endian token whose low
//! eleven bits encode distance minus one and whose high five bits encode length
//! minus three. Matches copy forward, including overlap.
use crate::{copy_match, grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Reader, Result};

/// Decode a mode-prefixed stream within an explicit output budget.
///
/// The stream begins with a mode byte and a little-endian 16-bit output count.
/// Mode zero copies that many literal bytes. Any nonzero mode selects grouped
/// compressed commands. Compressed matches are emitted in full before the
/// 16-bit remaining count is checked again, matching the source control flow;
/// `max_output` bounds such output. A zero declared count consumes only the
/// three-byte header. Trailing bytes remain unconsumed.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    decode_with_history(src, &[], max_output)
}

/// Decode with explicitly supplied bytes immediately preceding the output.
/// No implicit fill or unknown prehistory is assumed. At most 2048 preceding
/// bytes are addressable by this grammar.
pub fn decode_with_history(src: &[u8], history: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut reader = Reader::new(src);
    let mode = reader.u8()?;
    let mut remaining = reader.u16_le()?;
    let declared = usize::from(remaining);
    grow(&[], declared, limit)?;
    let mut out = Vec::with_capacity(declared);

    if mode == 0 {
        let raw = reader.bytes(declared)?;
        out.extend_from_slice(raw);
        return Ok(DecodeOutcome {
            bytes: out,
            consumed: reader.pos(),
        });
    }

    let history = &history[history.len().saturating_sub(2048)..];
    while remaining != 0 {
        let mut flags = reader.u16_le()?;
        for _ in 0..16 {
            if remaining == 0 {
                break;
            }
            if flags & 1 == 0 {
                grow(&out, 1, limit)?;
                out.push(reader.u8()?);
                remaining = remaining.wrapping_sub(1);
            } else {
                let token = usize::from(reader.u16_le()?);
                let distance = (token & 0x07ff) + 1;
                let length = (token >> 11) + 3;
                grow(&out, length, limit)?;
                copy_match(&mut out, history, distance, length, limit)?;
                remaining = remaining.wrapping_sub(length as u16);
            }
            flags >>= 1;
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: reader.pos(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use formatkit_core::Error;

    #[test]
    fn header_mode_literal_and_word_flag_order() {
        let raw = [0, 3, 0, b'A', b'B', b'C', 99];
        let decoded = decode(&raw, 3).unwrap();
        assert_eq!(decoded.bytes, b"ABC");
        assert_eq!(decoded.consumed, 6);

        let compressed = [1, 1, 0, 0, 0, b'Q'];
        let decoded = decode(&compressed, 1).unwrap();
        assert_eq!(decoded.bytes, b"Q");
        assert_eq!(decoded.consumed, compressed.len());
    }

    #[test]
    fn minimum_distance_overlap_and_token_field_split() {
        let stream = [1, 5, 0, 2, 0, b'A', 0, 8];
        let decoded = decode(&stream, 5).unwrap();
        assert_eq!(decoded.bytes, b"AAAAA");
        assert_eq!(decoded.consumed, stream.len());
    }

    #[test]
    fn full_word_flags_refill_lazily() {
        let mut stream = vec![1, 16, 0, 0, 0];
        stream.extend(0..16);
        stream.extend_from_slice(&[99, 100]);
        let decoded = decode(&stream, 16).unwrap();
        assert_eq!(decoded.bytes, (0..16).collect::<Vec<_>>());
        assert_eq!(decoded.consumed, 21);
    }

    #[test]
    fn explicit_history_and_unknown_history_refusal() {
        let stream = [1, 3, 0, 1, 0, 0, 0];
        assert!(decode(&stream, 3).is_err());
        assert_eq!(decode_with_history(&stream, b"Z", 3).unwrap().bytes, b"ZZZ");

        let mut history = vec![b'X'; 2048];
        history[0] = b'Y';
        let far = [1, 3, 0, 1, 0, 0xff, 0x07];
        assert_eq!(
            decode_with_history(&far, &history, 3).unwrap().bytes,
            b"YXX"
        );
    }

    #[test]
    fn source_does_not_clamp_a_final_match_to_the_declared_count() {
        // Declared count one, followed by a three-byte distance-one match.
        // The source decrements its 16-bit count for all three bytes, then
        // continues until zero; the bounded API must refuse the over-budget
        // continuation rather than return a silently clamped one-byte result.
        let mut stream = vec![1, 1, 0];
        for group_start in (0..1928).step_by(16) {
            let count = (1928 - group_start).min(16);
            let flags = if count == 16 {
                u16::MAX
            } else {
                (1u16 << count) - 1
            };
            stream.extend_from_slice(&flags.to_le_bytes());
            for token_index in group_start..group_start + count {
                let token = if token_index == 1926 {
                    0x8000u16
                } else {
                    0xf800u16
                };
                stream.extend_from_slice(&token.to_le_bytes());
            }
        }
        assert!(matches!(
            decode_with_history(&stream, b"Q", MAX_DECODED),
            Err(Error::ResourceLimit { .. })
        ));
    }

    #[test]
    fn zero_declared_size_truncation_and_resource_bounds() {
        assert_eq!(decode(&[1, 0, 0], 0).unwrap().consumed, 3);
        assert!(matches!(
            decode(&[1, 1, 0], 1),
            Err(Error::Truncated { .. })
        ));
        assert!(matches!(
            decode(&[1, 1, 0, 0, 0], 1),
            Err(Error::Truncated { offset: 5, .. })
        ));
        assert!(matches!(
            decode(&[1, 3, 0, 1, 0, 0], 3),
            Err(Error::Truncated { offset: 5, .. })
        ));
        assert!(matches!(
            decode(&[1, 2, 0], 1),
            Err(Error::ResourceLimit { .. })
        ));
    }
}
