//! Static binary Huffman stream with an external relative-offset tree table.
//!
//! Bitstream:
//! - Little-endian 16-bit words consumed LSB-first (bit 0 through bit 15).
//!
//! Tree table layout (byte array):
//! - At byte index `node`:
//!   - `tree[node] == 0`: leaf node; the literal symbol is `tree[node + 1]` (2 bytes).
//!   - `tree[node] != 0`: internal node; relative byte offset to child node (1 byte).
//! - For bit `0` (left child): evaluate `tree[node]`.
//! - For bit `1` (right child): advance past the left child entry (`+2` if left is a leaf,
//!   `+1` otherwise), then evaluate `tree[node]`.
//! - When a leaf is reached, emit its literal byte and reset `node = 0`.
use crate::{grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Result, Writer};

const MAX_CODE_BITS: u8 = 64;

/// Decode a static Huffman stream against an external relative-offset tree table.
///
/// When `decoded_size` is `Some(n)`, decoding stops as soon as `n` bytes are produced.
/// When `decoded_size` is `None`, all complete 16-bit words in `src` are decoded and
/// `consumed` equals `src.len()`.
pub fn decode(
    src: &[u8],
    tree: &[u8],
    decoded_size: Option<usize>,
    max_output: usize,
) -> Result<DecodeOutcome> {
    if tree.len() < 2 {
        return Err(Error::InvalidField {
            what: "huffman-static tree table too short",
            value: tree.len() as u64,
        });
    }
    let limit = max_output.min(MAX_DECODED);
    if let Some(target) = decoded_size {
        grow(&[], target, limit)?;
        if target == 0 {
            return Ok(DecodeOutcome {
                bytes: Vec::new(),
                consumed: 0,
            });
        }
    }

    let mut out = decoded_size.map_or_else(Vec::new, Vec::with_capacity);
    let mut node = 0usize;
    let mut si = 0usize;
    let tlen = tree.len();

    while si + 2 <= src.len() {
        let mut word = u16::from_le_bytes([src[si], src[si + 1]]);
        si += 2;
        for _ in 0..16 {
            let bit = word & 1;
            word >>= 1;
            if node + 1 >= tlen {
                return Err(Error::InvalidField {
                    what: "huffman-static tree node out of bounds",
                    value: node as u64,
                });
            }
            let b = tree[node];
            if bit == 0 {
                if b != 0 {
                    node += usize::from(b);
                    continue;
                }
            } else {
                node += if b == 0 { 2 } else { 1 };
                if node + 1 >= tlen {
                    return Err(Error::InvalidField {
                        what: "huffman-static right child out of bounds",
                        value: node as u64,
                    });
                }
                let rb = tree[node];
                if rb != 0 {
                    node += usize::from(rb);
                    continue;
                }
            }

            grow(&out, 1, limit)?;
            out.push(tree[node + 1]);
            node = 0;

            if decoded_size.is_some_and(|target| out.len() == target) {
                return Ok(DecodeOutcome {
                    bytes: out,
                    consumed: si,
                });
            }
        }
    }

    if let Some(target) = decoded_size {
        if out.len() < target {
            return Err(Error::Truncated {
                offset: src.len(),
                needed: 2,
                available: src.len() - si,
            });
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: src.len(),
    })
}

fn build_code_table(tree: &[u8]) -> Result<[Option<(u64, u8)>; 256]> {
    if tree.len() < 2 {
        return Err(Error::InvalidField {
            what: "huffman-static tree table too short",
            value: tree.len() as u64,
        });
    }
    let mut table: [Option<(u64, u8)>; 256] = [None; 256];
    let mut stack: Vec<(usize, u64, u8)> = vec![(0, 0, 0)];

    while let Some((node, bits, depth)) = stack.pop() {
        if depth >= MAX_CODE_BITS {
            return Err(Error::InvalidField {
                what: "huffman-static tree depth exceeds 64 bits",
                value: u64::from(depth),
            });
        }
        if node + 1 >= tree.len() {
            return Err(Error::InvalidField {
                what: "huffman-static tree node out of bounds",
                value: node as u64,
            });
        }

        // Left child (bit = 0)
        let left_b = tree[node];
        let left_depth = depth + 1;
        if left_b == 0 {
            let sym = usize::from(tree[node + 1]);
            if table[sym].is_none() {
                table[sym] = Some((bits, left_depth));
            }
        } else {
            let next = node + usize::from(left_b);
            if next >= tree.len() {
                return Err(Error::InvalidField {
                    what: "huffman-static left child out of bounds",
                    value: next as u64,
                });
            }
            stack.push((next, bits, left_depth));
        }

        // Right child (bit = 1)
        let right_node = node + if left_b == 0 { 2 } else { 1 };
        if right_node + 1 >= tree.len() {
            return Err(Error::InvalidField {
                what: "huffman-static right child out of bounds",
                value: right_node as u64,
            });
        }
        let right_bits = bits | (1u64 << depth);
        let right_b = tree[right_node];
        if right_b == 0 {
            let sym = usize::from(tree[right_node + 1]);
            if table[sym].is_none() {
                table[sym] = Some((right_bits, left_depth));
            }
        } else {
            let next = right_node + usize::from(right_b);
            if next >= tree.len() {
                return Err(Error::InvalidField {
                    what: "huffman-static right child out of bounds",
                    value: next as u64,
                });
            }
            stack.push((next, right_bits, left_depth));
        }
    }

    Ok(table)
}

/// Deterministic encoder for a byte stream using an external static Huffman tree table.
pub fn encode(src: &[u8], tree: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    let codes = build_code_table(tree)?;
    let mut w = Writer::new();
    let mut acc = 0u16;
    let mut bit_pos = 0u8;

    for &b in src {
        let (code, len) = codes[usize::from(b)].ok_or(Error::InvalidField {
            what: "huffman-static symbol missing from tree",
            value: u64::from(b),
        })?;
        for i in 0..len {
            if (code >> i) & 1 != 0 {
                acc |= 1u16 << bit_pos;
            }
            bit_pos += 1;
            if bit_pos == 16 {
                w.u16_le(acc);
                acc = 0;
                bit_pos = 0;
            }
        }
    }
    if bit_pos > 0 {
        w.u16_le(acc);
    }
    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic 4-symbol static tree:
    /// - bit 0 -> leaf 0x41 ('A')
    /// - bit 1,0 -> leaf 0x42 ('B')
    /// - bit 1,1,0 -> leaf 0x43 ('C')
    /// - bit 1,1,1 -> leaf 0x44 ('D')
    fn sample_tree() -> Vec<u8> {
        // node 0: left = leaf(0, 'A') [0..2], right = rel +1 -> node 3 [2]
        // node 3: left = leaf(0, 'B') [3..5], right = rel +1 -> node 6 [5]
        // node 6: left = leaf(0, 'C') [6..8], right = leaf(0, 'D') [8..10]
        vec![0, b'A', 1, 0, b'B', 1, 0, b'C', 0, b'D']
    }

    #[test]
    fn encodes_and_decodes_with_exact_length() {
        let tree = sample_tree();
        let payload = b"ABACADABBAABCDDCBA";
        let enc = encode(payload, &tree).unwrap();
        let dec = decode(&enc, &tree, Some(payload.len()), MAX_DECODED).unwrap();
        assert_eq!(dec.bytes, payload);
        assert_eq!(dec.consumed, enc.len());
    }

    #[test]
    fn carrier_bounded_accepts_odd_trailing_pad_byte() {
        let tree = sample_tree();
        let payload = b"AAAAAAAAAAAAAAAA"; // 16 'A's -> 16 zero bits -> 1 word (2 bytes)
        let mut enc = encode(payload, &tree).unwrap();
        assert_eq!(enc.len(), 2);
        enc.push(0xff); // trailing odd pad byte in carrier slice
        let dec = decode(&enc, &tree, None, MAX_DECODED).unwrap();
        assert_eq!(dec.bytes, payload);
        assert_eq!(dec.consumed, 3);
    }

    #[test]
    fn rejects_invalid_tree_and_missing_symbols() {
        let tree = sample_tree();
        assert!(matches!(
            encode(b"Z", &tree),
            Err(Error::InvalidField { .. })
        ));
        assert!(matches!(
            decode(&[0, 0], &[0], None, 64),
            Err(Error::InvalidField { .. })
        ));
        // OOB relative offset in tree
        assert!(matches!(
            decode(&[0, 0], &[99, 0], None, 64),
            Err(Error::InvalidField { .. })
        ));
    }
}
