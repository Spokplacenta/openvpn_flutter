//! Rust library for managing OpenVPN on Windows via FFI
//! 
//! This library exposes FFI functions to be called from Dart
//! via the Flutter Windows plugin.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

mod openvpn;
mod error;
mod manager;

pub use openvpn::{OpenVpnManager, VpnStats};
pub use error::*;
pub use manager::*;

/// Structure to represent VPN state
#[repr(C)]
pub struct VpnState {
    pub stage: *const c_char,  // Current stage (connected, disconnected, etc.)
    pub connected_on: *const c_char,  // ISO8601 connection timestamp
    pub byte_in: u64,
    pub byte_out: u64,
    pub packets_in: u64,
    pub packets_out: u64,
}

impl Default for VpnState {
    fn default() -> Self {
        VpnState {
            stage: ptr::null(),
            connected_on: ptr::null(),
            byte_in: 0,
            byte_out: 0,
            packets_in: 0,
            packets_out: 0,
        }
    }
}

/// Initializes the OpenVPN manager
/// 
/// # Safety
/// This function is unsafe because it manipulates C pointers
#[no_mangle]
pub unsafe extern "C" fn openvpn_initialize(
    binary_path: *const c_char,
) -> c_int {
    if binary_path.is_null() {
        return -1; // Error: null path
    }

    let path = match CStr::from_ptr(binary_path).to_str() {
        Ok(s) => s,
        Err(_) => return -2, // Error: UTF-8 conversion
    };

    match manager::init_manager(path.to_string()) {
        Ok(_) => 0, // Success
        Err(_) => -3, // Error: initialization failed
    }
}

/// Connects to VPN with the provided configuration
/// 
/// # Safety
/// This function is unsafe because it manipulates C pointers
#[no_mangle]
pub unsafe extern "C" fn openvpn_connect(
    config: *const c_char,
    username: *const c_char,
    password: *const c_char,
) -> c_int {
    if config.is_null() {
        return -1; // Error: null config
    }

    let config_str = match CStr::from_ptr(config).to_str() {
        Ok(s) => s,
        Err(_) => return -2,
    };

    let username_str = if !username.is_null() {
        match CStr::from_ptr(username).to_str() {
            Ok(s) => Some(s.to_string()),
            Err(_) => return -3,
        }
    } else {
        None
    };

    let password_str = if !password.is_null() {
        match CStr::from_ptr(password).to_str() {
            Ok(s) => Some(s.to_string()),
            Err(_) => return -4,
        }
    } else {
        None
    };

    // Get the global manager
    let manager_arc = match manager::get_manager() {
        Ok(m) => m,
        Err(_) => return -5, // Manager not initialized
    };

    let manager_guard = manager_arc.lock().unwrap();
    if let Some(ref manager) = *manager_guard {
        // Create a Tokio runtime to execute the async function
        let rt = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(_) => return -6, // Runtime creation error
        };

        match rt.block_on(manager.connect(config_str, username_str, password_str)) {
            Ok(_) => 0,
            Err(_) => -7, // Connection error
        }
    } else {
        -5 // Manager not initialized
    }
}

/// Disconnects from VPN
/// 
/// # Safety
/// This function is unsafe because it manipulates C pointers
#[no_mangle]
pub unsafe extern "C" fn openvpn_disconnect() -> c_int {
    // Get the global manager
    let manager_arc = match manager::get_manager() {
        Ok(m) => m,
        Err(_) => return -1, // Manager not initialized
    };

    let manager_guard = manager_arc.lock().unwrap();
    if let Some(ref manager) = *manager_guard {
        // Create a Tokio runtime to execute the async function
        let rt = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(_) => return -2, // Runtime creation error
        };

        match rt.block_on(manager.disconnect()) {
            Ok(_) => 0,
            Err(_) => -3, // Disconnection error
        }
    } else {
        -1 // Manager not initialized
    }
}

/// Gets the current VPN stage
/// 
/// Returns an allocated C string that must be freed with openvpn_free_string
/// 
/// # Safety
/// This function is unsafe because it manipulates C pointers
#[no_mangle]
pub unsafe extern "C" fn openvpn_get_stage() -> *mut c_char {
    // Get the global manager
    let manager_arc = match manager::get_manager() {
        Ok(m) => m,
        Err(_) => {
            // If manager is not initialized, return "disconnected"
            return CString::new("disconnected").unwrap().into_raw();
        }
    };

    let manager_guard = manager_arc.lock().unwrap();
    if let Some(ref manager) = *manager_guard {
        match manager.get_stage() {
            Ok(stage) => {
                CString::new(stage).unwrap_or_else(|_| {
                    CString::new("disconnected").unwrap()
                }).into_raw()
            },
            Err(_) => {
                CString::new("disconnected").unwrap().into_raw()
            }
        }
    } else {
        CString::new("disconnected").unwrap().into_raw()
    }
}

/// Gets the current VPN status
/// 
/// Returns a VpnState structure that must be freed with openvpn_free_state
/// 
/// # Safety
/// This function is unsafe because it manipulates C pointers
#[no_mangle]
pub unsafe extern "C" fn openvpn_get_status() -> *mut VpnState {
    // Get the global manager
    let manager_arc = match manager::get_manager() {
        Ok(m) => m,
        Err(_) => {
            // If manager is not initialized, return an empty state
            let state = Box::new(VpnState::default());
            return Box::into_raw(state);
        }
    };

    let manager_guard = manager_arc.lock().unwrap();
    if let Some(ref manager) = *manager_guard {
        match manager.get_stats() {
            Ok(stats) => {
                // Convert VpnStats to VpnState
                let stage_str = manager.get_stage().unwrap_or_else(|_| "disconnected".to_string());
                let stage_cstr = CString::new(stage_str).unwrap_or_else(|_| {
                    CString::new("disconnected").unwrap()
                });
                
                let connected_on_str = if let Some(connected_on) = stats.connected_on {
                    // Convert SystemTime to ISO8601 (RFC3339)
                    match connected_on.duration_since(std::time::UNIX_EPOCH) {
                        Ok(duration) => {
                            let datetime = chrono::DateTime::<chrono::Utc>::from_timestamp(
                                duration.as_secs() as i64,
                                duration.subsec_nanos(),
                            );
                            datetime.map(|dt| dt.to_rfc3339()).unwrap_or_default()
                        },
                        Err(_) => String::new(),
                    }
                } else {
                    String::new()
                };
                
                let connected_on_cstr = if connected_on_str.is_empty() {
                    ptr::null()
                } else {
                    match CString::new(connected_on_str) {
                        Ok(s) => s.into_raw() as *const c_char,
                        Err(_) => ptr::null(),
                    }
                };
                
                // Convert stage_cstr to raw pointer for storage
                let stage_ptr = stage_cstr.into_raw();
                
                let state = Box::new(VpnState {
                    stage: stage_ptr as *const c_char,
                    connected_on: connected_on_cstr,
                    byte_in: stats.byte_in,
                    byte_out: stats.byte_out,
                    packets_in: stats.packets_in,
                    packets_out: stats.packets_out,
                });
                
                Box::into_raw(state)
            },
            Err(_) => {
                // On error, return default state
                let state = Box::new(VpnState::default());
                Box::into_raw(state)
            }
        }
    } else {
        // Manager not initialized
        let state = Box::new(VpnState::default());
        Box::into_raw(state)
    }
}

/// Frees a C string allocated by the library
/// 
/// # Safety
/// This function is unsafe because it manipulates C pointers
#[no_mangle]
pub unsafe extern "C" fn openvpn_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        let _ = CString::from_raw(ptr);
    }
}

/// Frees a VpnState structure allocated by the library
/// 
/// # Safety
/// This function is unsafe because it manipulates C pointers
#[no_mangle]
pub unsafe extern "C" fn openvpn_free_state(ptr: *mut VpnState) {
    if !ptr.is_null() {
        let state = Box::from_raw(ptr);
        
        // Free C strings if they are not null
        if !state.stage.is_null() {
            // Free the stage string
            let _ = CString::from_raw(state.stage as *mut c_char);
        }
        
        if !state.connected_on.is_null() {
            // Free the connected_on string
            let _ = CString::from_raw(state.connected_on as *mut c_char);
        }
    }
}

