//! Dynamic-link boundary for `Experimental_CreateProcessInSandbox`.
//!
//! All experimental Microsoft types stay inside this module. The DLL is
//! loaded from System32 only and never freed, matching the process-lifetime
//! caching contract used for backend probing.

#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{GetLastError, FARPROC};
#[cfg(windows)]
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{PROCESS_INFORMATION, STARTUPINFOW};

#[cfg(windows)]
pub(crate) type PfnCreateProcessInSandbox = unsafe extern "system" fn(
    application_name: *const u16,
    command_line: *mut u16,
    process_attributes: *const core::ffi::c_void,
    thread_attributes: *const core::ffi::c_void,
    inherit_handles: i32,
    creation_flags: u32,
    environment: *const core::ffi::c_void,
    current_directory: *const u16,
    startup_info: *const STARTUPINFOW,
    identity: *const u16,
    sandbox_specification: *const u8,
    sandbox_specification_size: u32,
    process_information: *mut PROCESS_INFORMATION,
) -> i32;

/// `Experimental_QuerySandboxSupport`, when present. Writes a `SANDBOX_CAP_*`
/// bitmask and returns non-zero on success.
#[cfg(windows)]
pub(crate) type PfnQuerySandboxSupport = unsafe extern "system" fn(capabilities: *mut u64) -> i32;

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const SANDBOX_CAP_CREATE_PROCESS_IN_SANDBOX: u64 = 0x0000_0000_0000_0001;

#[cfg(windows)]
static CREATE_API: OnceLock<Result<PfnCreateProcessInSandbox, String>> = OnceLock::new();
#[cfg(windows)]
static QUERY_API: OnceLock<Option<PfnQuerySandboxSupport>> = OnceLock::new();

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(not(windows))]
pub(crate) fn create_api() -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
pub(crate) fn create_api() -> Result<PfnCreateProcessInSandbox, String> {
    match CREATE_API.get_or_init(load_create_api) {
        Ok(api) => Ok(*api),
        Err(err) => Err(err.clone()),
    }
}

#[cfg(windows)]
fn load_create_api() -> Result<PfnCreateProcessInSandbox, String> {
    let dll = to_wide("processmodel.dll");
    // SAFETY: The DLL name is a valid null-terminated wide string that outlives
    // the call, and LOAD_LIBRARY_SEARCH_SYSTEM32 restricts the search to
    // System32. The module is deliberately never freed; GetProcAddress results
    // stay valid for the process lifetime.
    unsafe {
        let module = LoadLibraryExW(
            dll.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if module.is_null() {
            return Err(format!(
                "LoadLibraryExW(processmodel.dll) failed, Win32 error {}",
                GetLastError()
            ));
        }

        let export = b"Experimental_CreateProcessInSandbox\0";
        let symbol = GetProcAddress(module, export.as_ptr().cast());
        let Some(symbol) = symbol else {
            return Err(
                "GetProcAddress(Experimental_CreateProcessInSandbox) failed: \
                 API not present on this OS build"
                    .into(),
            );
        };

        Ok(std::mem::transmute::<FARPROC, PfnCreateProcessInSandbox>(
            Some(symbol),
        ))
    }
}

#[cfg(windows)]
pub(crate) fn query_sandbox_capabilities() -> Option<u64> {
    let query = match QUERY_API.get_or_init(load_query_api) {
        Some(query) => *query,
        None => return None,
    };
    let mut capabilities = 0u64;
    // SAFETY: `query` is a resolved export and `capabilities` is a valid
    // out-parameter for the documented function signature.
    let ok = unsafe { query(&mut capabilities) };
    if ok == 0 {
        None
    } else {
        Some(capabilities)
    }
}

#[cfg(windows)]
fn load_query_api() -> Option<PfnQuerySandboxSupport> {
    let dll = to_wide("processmodel.dll");
    // SAFETY: Same loading contract as `load_create_api`.
    unsafe {
        let module = LoadLibraryExW(
            dll.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if module.is_null() {
            return None;
        }
        let export = b"Experimental_QuerySandboxSupport\0";
        let symbol = GetProcAddress(module, export.as_ptr().cast())?;
        Some(std::mem::transmute::<FARPROC, PfnQuerySandboxSupport>(
            Some(symbol),
        ))
    }
}
