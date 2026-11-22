//! Safe wrappers for the Windows interfaces used by the toolkit.
#![cfg(windows)]

use std::{fs::File, io, os::windows::io::AsRawHandle, path::Path};
use windows_sys::Win32::{
    Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG},
    Storage::FileSystem::{FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO},
};

/// Return the volume serial number and the full file identifier.
pub fn file_id(path: &Path) -> io::Result<(u64, u128)> {
    let file = File::open(path)?;
    let mut identity = FILE_ID_INFO::default();
    // The handle stays open and the output buffer is a writable FILE_ID_INFO.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut identity as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((
        identity.VolumeSerialNumber,
        u128::from_le_bytes(identity.FileId.Identifier),
    ))
}

/// Fill the buffer from the system's preferred cryptographic RNG.
pub fn fill_random(buffer: &mut [u8]) -> io::Result<()> {
    for chunk in buffer.chunks_mut(u32::MAX as usize) {
        // The provider requires a null handle and writes only to this slice.
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                chunk.as_mut_ptr(),
                chunk.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status != 0 {
            return Err(io::Error::other(format!("system RNG failed ({status:#x})")));
        }
    }
    Ok(())
}
