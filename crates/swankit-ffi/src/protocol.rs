//! Versioned JSON-lines subprocess protocol for the evaluation CLI.
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};

pub const MAX_LINE: usize = 16384;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    operation: u32,
    path: String,
}

pub fn response(line: &[u8]) -> Value {
    let result = (|| -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
        let request: Request = serde_json::from_slice(line)?;
        if request.version != 1 {
            return Err("unsupported protocol version".into());
        }
        let file = std::fs::File::open(request.path)?;
        if file.metadata()?.len() > super::MAX_INPUT as u64 {
            return Err("input exceeds interop bound".into());
        }
        let mut input = Vec::new();
        file.take(super::MAX_INPUT as u64 + 1)
            .read_to_end(&mut input)?;
        Ok(super::decode(request.operation, &input)?)
    })();
    match result {
        Ok(output) => json!({"version":1,"output":output}),
        Err(error) => json!({"version":1,"error":error.to_string()}),
    }
}

/// Bound allocation before reading a frame. Oversize frames close the stream;
/// invalid JSON/operations/files receive errors and allow the next request.
pub fn serve(input: impl BufRead, mut output: impl Write, one: bool) -> std::io::Result<()> {
    let mut input = input;
    loop {
        let mut line = Vec::new();
        let count = input
            .by_ref()
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        let oversized = count > MAX_LINE;
        let value = if oversized {
            json!({"version":1,"error":"request line exceeds bound"})
        } else {
            response(&line)
        };
        serde_json::to_writer(&mut output, &value)?;
        output.write_all(b"\n")?;
        output.flush()?;
        if one || oversized {
            break;
        }
    }
    Ok(())
}
