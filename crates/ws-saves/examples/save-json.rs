//! Local inspection utility; all filesystem access is at this example boundary.
use std::{
    io::{self, BufRead},
    path::Path,
};
fn main() {
    for line in io::stdin().lock().lines() {
        let result = (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
            let line = line?;
            let request: serde_json::Value = serde_json::from_str(&line)?;
            let id = request["id"].as_str().ok_or("id missing")?;
            let path = request["path"].as_str().ok_or("path missing")?;
            let mut bytes = std::fs::read(path)?;
            let format = ws_saves::format(id)?;
            let name = Path::new(path)
                .file_name()
                .ok_or("filename missing")?
                .to_string_lossy();
            if let Some(offset) = request["corrupt_offset"].as_u64() {
                let b = bytes
                    .get_mut(usize::try_from(offset)?)
                    .ok_or("corruption offset")?;
                *b ^= 1;
                format.fix(&mut bytes, &name)?;
            }
            if let Some(output) = request["fix_output"].as_str() {
                format.fix(&mut bytes, &name)?;
                std::fs::write(output, &bytes)?;
            }
            Ok(format.decode(&bytes, &name)?)
        })();
        println!(
            "{}",
            match result {
                Ok(value) => serde_json::json!({"value":value}),
                Err(error) => serde_json::json!({"error":error.to_string()}),
            }
        );
    }
}
