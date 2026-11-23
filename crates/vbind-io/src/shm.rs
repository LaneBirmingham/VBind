//! Named shared memory, the channel game integrations use to reach a
//! game-side add-on.

use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::core::HSTRING;

/// A mapped view of a named shared-memory block. The block exists while any
/// process (VBind or the game) holds it open.
pub struct SharedMemory {
    name: String,
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    size: usize,
}

// The view is plain memory; access goes through &self / &mut self.
unsafe impl Send for SharedMemory {}

impl SharedMemory {
    /// Create the block, or open it when the game made it first.
    pub fn create(name: &str, size: u32) -> Result<SharedMemory, String> {
        let wide = HSTRING::from(name);
        let handle = unsafe { CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, size, &wide) }
            .map_err(|e| format!("cannot create shared memory '{name}': {}", e.message()))?;
        Self::map(name, handle, size, FILE_MAP_ALL_ACCESS.0)
    }

    /// Open an existing block. `Ok(None)` when nobody has made it.
    pub fn open(name: &str, size: u32, write: bool) -> Result<Option<SharedMemory>, String> {
        let wide = HSTRING::from(name);
        let access = if write { FILE_MAP_ALL_ACCESS.0 } else { FILE_MAP_READ.0 };
        match unsafe { OpenFileMappingW(access, false, &wide) } {
            Ok(handle) => Self::map(name, handle, size, access).map(Some),
            Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => Ok(None),
            Err(e) => Err(format!("cannot open shared memory '{name}': {}", e.message())),
        }
    }

    fn map(name: &str, handle: HANDLE, size: u32, access: u32) -> Result<SharedMemory, String> {
        let view = unsafe { MapViewOfFile(handle, windows::Win32::System::Memory::FILE_MAP(access), 0, 0, size as usize) };
        if view.Value.is_null() {
            let e = windows::core::Error::from_thread();
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(format!("cannot map {size} bytes of shared memory '{name}' (is it smaller?): {}", e.message()));
        }
        Ok(SharedMemory { name: name.to_string(), handle, view, size: size as usize })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Write `bytes` from the start of the block. Extra bytes are ignored.
    pub fn write(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.size);
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.view.Value as *mut u8, n) };
    }

    /// Fill the block with zeros.
    pub fn clear(&mut self) {
        unsafe { std::ptr::write_bytes(self.view.Value as *mut u8, 0, self.size) };
    }

    pub fn read(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.size];
        unsafe { std::ptr::copy_nonoverlapping(self.view.Value as *const u8, out.as_mut_ptr(), self.size) };
        out
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Read a block if it exists.
pub fn read_existing(name: &str, size: u32) -> Result<Option<Vec<u8>>, String> {
    Ok(SharedMemory::open(name, size, false)?.map(|m| m.read()))
}

/// Zero a block if it exists, so a game never keeps acting on input left by
/// a VBind that stopped without cleaning up.
pub fn clear_existing(name: &str, size: u32) -> Result<bool, String> {
    match SharedMemory::open(name, size, true)? {
        Some(mut m) => {
            m.clear();
            Ok(true)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_between_handles() {
        let name = format!("Local\\VBind_Test_{}", std::process::id());
        assert!(read_existing(&name, 16).unwrap().is_none());
        let mut a = SharedMemory::create(&name, 16).unwrap();
        a.write(&[1, 2, 3]);
        let b = read_existing(&name, 16).unwrap().unwrap();
        assert_eq!(&b[..4], &[1, 2, 3, 0]);
        assert!(clear_existing(&name, 16).unwrap());
        assert_eq!(a.read(), vec![0; 16]);
        drop(a);
        assert!(read_existing(&name, 16).unwrap().is_none(), "gone once every handle closes");
    }
}
