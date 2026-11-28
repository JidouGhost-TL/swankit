//! Zero-escaped byte RLE stream for 8-bit PCM payloads.
//! Non-zero bytes are emitted literally; `0x00 <val> <count>` repeats `val`
//! `count` times (`count == 0` encodes 256). Extent is supplied by the carrier.
use crate::{grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::{Reader, Result, Writer};

/// Decode a carrier-bounded zero-escaped RLE stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_STREAM_DECODED);
    let mut r = Reader::new(src);
    let mut out = Vec::new();
    while r.remaining() > 0 {
        let b = r.u8()?;
        if b == 0 {
            let value = r.u8()?;
            let count = r.u8()?;
            let repeat = if count == 0 { 256 } else { usize::from(count) };
            grow(&out, repeat, limit)?;
            out.resize(out.len() + repeat, value);
        } else {
            grow(&out, 1, limit)?;
            out.push(b);
        }
    }
    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic encoder for zero-escaped byte RLE streams.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_STREAM_DECODED)?;
    let mut w = Writer::new();
    let n = src.len();
    let mut i = 0;
    while i < n {
        let b = src[i];
        let mut run = 1usize;
        while i + run < n && run < 256 && src[i + run] == b {
            run += 1;
        }
        if b == 0 || run >= 4 {
            w.u8(0);
            w.u8(b);
            w.u8(if run == 256 { 0 } else { run as u8 });
            i += run;
        } else {
            w.u8(b);
            i += 1;
        }
    }
    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_literals_zero_escapes_and_256_runs() {
        let stream = [0x80, 0x81, 0x00, 0x80, 4, 0x00, 0x00, 1, 0x00, 0x7f, 0];
        let got = decode(&stream, 512).unwrap();
        let mut expected = vec![0x80, 0x81, 0x80, 0x80, 0x80, 0x80, 0x00];
        expected.resize(expected.len() + 256, 0x7f);
        assert_eq!(got.bytes, expected);
        assert_eq!(got.consumed, stream.len());
        assert_eq!(decode(&[], 0).unwrap().bytes, Vec::<u8>::new());
        assert_eq!(encode(&[]).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn rejects_truncation_and_output_overflow() {
        assert!(decode(&[0x00], 16).is_err());
        assert!(decode(&[0x00, 0x80], 16).is_err());
        assert!(decode(&[0x80, 0x00], 16).is_err());
        assert!(decode(&[0x00, 0x80, 5], 4).is_err());
        assert!(decode(&[0x00, 0x80, 0], 255).is_err());
        assert!(encode(&vec![0x80; MAX_STREAM_DECODED + 1]).is_err());
    }

    #[test]
    fn round_trips_zero_runs_and_pcm_waveforms() {
        for raw in [
            vec![],
            vec![0x00],
            vec![0x00; 256],
            vec![0x00; 257],
            vec![0x80; 3],
            vec![0x80; 4],
            vec![0x80; 256],
            vec![0x80; 257],
            (0..=255u8).collect::<Vec<_>>(),
        ] {
            let encoded = encode(&raw).unwrap();
            let got = decode(&encoded, raw.len()).unwrap();
            assert_eq!(got.bytes, raw);
            assert_eq!(got.consumed, encoded.len());
            assert_eq!(encode(&raw).unwrap(), encoded);
        }
        let wave: Vec<u8> = (0..0x18000)
            .map(|i| if i % 17 == 0 { 0 } else { (128 + (i % 9)) as u8 })
            .collect();
        let encoded = encode(&wave).unwrap();
        let got = decode(&encoded, wave.len()).unwrap();
        assert_eq!(got.bytes, wave);
        assert_eq!(got.consumed, encoded.len());
    }
}
