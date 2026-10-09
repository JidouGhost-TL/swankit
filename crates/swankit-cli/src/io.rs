use crate::Result;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub fn read(path: &str, maximum: usize) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|e| format!("read {path}: {e}"))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(format!("input is not a regular file: {path}").into());
    }
    if metadata.len() > maximum as u64 {
        return Err(format!("input exceeds {maximum} bytes: {path}").into());
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(format!("input grew beyond {maximum} bytes: {path}").into());
    }
    Ok(bytes)
}

pub fn write(path: &str, bytes: &[u8], force: bool) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options
        .open(Path::new(path))
        .map_err(|e| format!("write {path}: {e}; use --force to replace an existing output"))?;
    file.write_all(bytes)?;
    file.flush()?;
    Ok(())
}
