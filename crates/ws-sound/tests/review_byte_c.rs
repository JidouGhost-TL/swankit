//! Driver-derived expectations and ROM sources remain in external specs.
use formatkit_core::{bytes_at, Result};
use formatkit_corpus_test::{run_dir, Args, Digest};
use serde_json::{json, Value};
use ws_sound::drivers::byte::{ByteCPcm, Dialect, Header};
struct ByteC;
impl Digest for ByteC {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let mut records = vec![];
        for row in args.get("headers").unwrap().as_array().unwrap() {
            let offset = row["offset"].as_u64().unwrap() as usize;
            let length = row["length"].as_u64().unwrap() as usize;
            let data = bytes_at(bytes, offset, length)?;
            let header = Header::parse(data, Dialect::C, 0)?;
            if header.mask == 0 {
                let pcm = ByteCPcm::parse(data)?;
                records.push(json!({"offset":offset,"mask":0,"bank":pcm.bank,"flags":pcm.flags,"offset_word":pcm.offset,"segment":pcm.segment,"length":pcm.length,"rate":pcm.rate,"repeat":pcm.repeat}));
            } else {
                records.push(json!({"offset":offset,"mask":header.mask,"priority":header.priority,"bank":header.bank,"segment":header.segment,"tracks":header.tracks.iter().map(|t| json!({"slot":t.slot,"offset":t.offset})).collect::<Vec<_>>()}));
            }
        }
        Ok(json!({"records":records}))
    }
}
#[test]
fn real_byte_c_headers_match_reconstructed_driver() {
    run_dir("ws-sound-byte-c", &ByteC);
}
