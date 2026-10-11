//! Explicit codec utility for external independent witnesses.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 4 {
        return Err("usage: stream decode|encode INPUT OUTPUT".into());
    }
    let input = std::fs::read(&a[2])?;
    let output = match a[1].as_str() {
        "decode" => ws_compress::lzss_u16_zero::decode(&input, 65536)?.bytes,
        "encode" => ws_compress::lzss_u16_zero::encode(&input)?,
        "encode12" => ws_compress::lzss_12::encode(&input)?,
        op if op.starts_with("decode12:") => {
            ws_compress::lzss_12::decode(&input, op[9..].parse()?, 65536)?.bytes
        }
        _ => return Err("unknown operation".into()),
    };
    std::fs::write(&a[3], output)?;
    Ok(())
}
