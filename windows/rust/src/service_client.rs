//! Client for the OpenVPN Interactive Service named pipe.
//!
//! Protocol (from interactive.c / openvpn-gui openvpn.c):
//!
//! 1. Open `\\.\pipe\openvpn\service` (GENERIC_READ | GENERIC_WRITE).
//! 2. Write startup data as a WCHAR buffer:
//!    `config_directory\0options\0management_password\n\0`
//!    (three null-separated wide-string fields, trailing null).
//! 3. Read response:
//!    - Success: `0x00000000\n0x<PID_hex>\nProcess ID`
//!    - Error:   `0x<error_code>\n<description>`

use crate::error::OpenVpnError;

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::io::{Read, Write};
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Pipes::SetNamedPipeHandleState;

    const PIPE_NAME: &str = r"\\.\pipe\openvpn\service";
    const PIPE_READMODE_MESSAGE: u32 = 0x00000002;

    fn wide_str(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Check if the Interactive Service pipe exists and can be opened.
    pub fn is_service_available() -> bool {
        let pipe_w = wide_str(PIPE_NAME);
        let handle = unsafe {
            CreateFileW(
                pipe_w.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return false;
        }
        unsafe { CloseHandle(handle) };
        true
    }

    /// Send startup data to the Interactive Service and return the PID of
    /// the openvpn.exe process it spawns.
    pub fn start_openvpn(
        config_dir: &str,
        options: &str,
        mgmt_password: &str,
    ) -> Result<u32, OpenVpnError> {
        let pipe_w = wide_str(PIPE_NAME);
        let handle = unsafe {
            CreateFileW(
                pipe_w.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0, // synchronous I/O for simplicity
                std::ptr::null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(OpenVpnError::ServiceError(
                "Cannot open Interactive Service pipe. Is the service running?".into(),
            ));
        }

        // Switch pipe to message-read mode.
        let mut mode: u32 = PIPE_READMODE_MESSAGE;
        let ok = unsafe {
            SetNamedPipeHandleState(handle, &mut mode, std::ptr::null_mut(), std::ptr::null_mut())
        };
        if ok == 0 {
            unsafe { CloseHandle(handle) };
            return Err(OpenVpnError::ServiceError(
                "Failed to set pipe read mode".into(),
            ));
        }

        // Build the startup data buffer as wide chars:
        // config_dir \0 options \0 mgmt_password\n \0
        let password_with_nl = format!("{}\n", mgmt_password);
        let mut data: Vec<u16> = Vec::new();
        data.extend(config_dir.encode_utf16());
        data.push(0);
        data.extend(options.encode_utf16());
        data.push(0);
        data.extend(password_with_nl.encode_utf16());
        data.push(0);

        // Write to pipe using std::fs::File wrapper for convenience.
        let mut pipe_file =
            unsafe { std::fs::File::from_raw_handle(handle as *mut std::ffi::c_void) };

        let data_bytes: Vec<u8> = data
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();

        pipe_file.write_all(&data_bytes).map_err(|e| {
            OpenVpnError::ServiceError(format!("Failed to write startup data to pipe: {}", e))
        })?;
        pipe_file.flush().map_err(|e| {
            OpenVpnError::ServiceError(format!("Failed to flush pipe: {}", e))
        })?;

        // Read response (wide char string).
        // The service responds with a message like:
        //   "0x00000000\n0x00001234\nProcess ID" on success
        //   "0x20000001\n..." on error
        let mut buf = vec![0u8; 4096];
        let bytes_read = pipe_file.read(&mut buf).map_err(|e| {
            OpenVpnError::ServiceError(format!("Failed to read service response: {}", e))
        })?;

        // Don't let File drop close the handle twice — it's already managed.
        // Actually, File::from_raw_handle takes ownership, so drop is fine.

        if bytes_read == 0 {
            return Err(OpenVpnError::ServiceError(
                "Empty response from Interactive Service".into(),
            ));
        }

        // Convert response from wide chars to String.
        let wide_slice: &[u16] = unsafe {
            std::slice::from_raw_parts(buf.as_ptr() as *const u16, bytes_read / 2)
        };
        let response = String::from_utf16_lossy(wide_slice);

        parse_service_response(&response)
    }

    fn parse_service_response(response: &str) -> Result<u32, OpenVpnError> {
        // First line: "0x%08x" error code.  0 = success.
        let mut lines = response.lines();
        let error_line = lines.next().unwrap_or("");

        let error_code = u32::from_str_radix(
            error_line.trim_start_matches("0x").trim_start_matches("0X"),
            16,
        )
        .unwrap_or(0xFFFFFFFF);

        if error_code != 0 {
            let rest: String = lines.collect::<Vec<_>>().join("\n");
            return Err(OpenVpnError::ServiceError(format!(
                "Interactive Service error 0x{:08x}: {}",
                error_code, rest
            )));
        }

        // Second line should contain the PID as "0x%08x".
        let pid_line = lines.next().unwrap_or("");
        let pid = u32::from_str_radix(
            pid_line.trim_start_matches("0x").trim_start_matches("0X"),
            16,
        )
        .map_err(|_| {
            OpenVpnError::ServiceError(format!(
                "Cannot parse PID from service response: {}",
                pid_line
            ))
        })?;

        if pid == 0 {
            return Err(OpenVpnError::ServiceError(
                "Service returned PID 0".into(),
            ));
        }

        Ok(pid)
    }
}

/// Public re-export for the service client.
pub struct ServicePipeClient;

impl ServicePipeClient {
    #[cfg(target_os = "windows")]
    pub fn is_service_available() -> bool {
        platform::is_service_available()
    }

    #[cfg(not(target_os = "windows"))]
    pub fn is_service_available() -> bool {
        false
    }

    #[cfg(target_os = "windows")]
    pub fn start_openvpn(
        config_dir: &str,
        options: &str,
        mgmt_password: &str,
    ) -> Result<u32, OpenVpnError> {
        platform::start_openvpn(config_dir, options, mgmt_password)
    }

    #[cfg(not(target_os = "windows"))]
    pub fn start_openvpn(
        _config_dir: &str,
        _options: &str,
        _mgmt_password: &str,
    ) -> Result<u32, OpenVpnError> {
        Err(OpenVpnError::ServiceError(
            "Interactive Service only available on Windows".into(),
        ))
    }
}
