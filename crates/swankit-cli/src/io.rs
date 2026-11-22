use crate::Result;
use std::{
    fs::{Metadata, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub fn read(path: &str, maximum: usize) -> Result<Vec<u8>> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(format!("input is not a regular file: {path}").into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|e| format!("read {path}: {e}"))?;
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
    let mut file = options.open(Path::new(path)).map_err(|e| {
        if !force && e.kind() == std::io::ErrorKind::AlreadyExists && Path::new(path).is_file() {
            format!("write {path}: {e}; use --force to replace an existing output")
        } else {
            format!("write {path}: {e}")
        }
    })?;
    file.write_all(bytes)?;
    file.flush()?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileIdentity(u64, u128);

#[cfg(unix)]
pub(crate) fn file_identity(_: &Path, metadata: &Metadata) -> std::io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    Ok(FileIdentity(metadata.dev(), u128::from(metadata.ino())))
}

#[cfg(windows)]
pub(crate) fn file_identity(path: &Path, _: &Metadata) -> std::io::Result<FileIdentity> {
    let (volume, identifier) = ws_platform::file_id(path)?;
    Ok(FileIdentity(volume, identifier))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn file_identity(_: &Path, _: &Metadata) -> std::io::Result<FileIdentity> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "file identity is unavailable on this platform",
    ))
}

/// Refuse a save output that resolves to an input, including filesystem aliases.
#[cfg(feature = "saves")]
pub fn ensure_distinct_path(output: &str, input: &str) -> Result<()> {
    let input_path = Path::new(input);
    let output_path = Path::new(output);
    let input_canonical = input_path.canonicalize()?;
    let output_canonical = if output_path.exists() {
        output_path.canonicalize()?
    } else {
        let parent = output_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        parent.canonicalize()?.join(
            output_path
                .file_name()
                .ok_or("output path has no filename")?,
        )
    };
    if input_canonical == output_canonical {
        return Err("save output aliases its input".into());
    }
    match std::fs::metadata(output_path) {
        Ok(output_metadata) => {
            if file_identity(input_path, &std::fs::metadata(input_path)?)?
                == file_identity(output_path, &output_metadata)?
            {
                return Err("save output aliases its input".into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
