//! OpenVPN process management on Windows
//!
//! Supports three launch modes:
//! 1. Interactive Service (preferred) — via named pipe to OpenVPNServiceInteractive
//! 2. UAC elevation — ShellExecuteEx with "runas" verb
//! 3. Direct spawn — legacy mode, requires running as admin

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tokio::process::{Child as TokioChild, Command as TokioCommand};

use crate::error::OpenVpnError;
use crate::management::{ManagementClient, find_free_port, generate_mgmt_password};
#[cfg(target_os = "windows")]
use crate::service_client::ServicePipeClient;

const ENABLE_DEBUG_LOGS: bool = false;

fn debug_log_to_file(msg: &str) {
    if !ENABLE_DEBUG_LOGS {
        return;
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::temp_dir().join("openvpn_rust_debug.log"))
    {
        let _ = writeln!(file, "{}", msg);
        let _ = file.flush();
    }
    eprintln!("{}", msg);
}

/// VPN connection statistics
#[derive(Clone, Default)]
pub struct VpnStats {
    pub byte_in: u64,
    pub byte_out: u64,
    pub packets_in: u64,
    pub packets_out: u64,
    pub connected_on: Option<std::time::SystemTime>,
}

/// How the OpenVPN process was launched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LaunchMode {
    /// Automatic: try service first, then fallback.
    Auto = 0,
    /// Via the OpenVPN Interactive Service named pipe.
    Service = 1,
    /// Direct spawn (current process must be admin).
    Direct = 2,
    /// Elevated via ShellExecuteEx "runas".
    Uac = 3,
}

impl From<i32> for LaunchMode {
    fn from(v: i32) -> Self {
        match v {
            1 => LaunchMode::Service,
            2 => LaunchMode::Direct,
            3 => LaunchMode::Uac,
            _ => LaunchMode::Auto,
        }
    }
}

/// Main OpenVPN manager
pub struct OpenVpnManager {
    binary_path: PathBuf,
    process: Arc<Mutex<Option<TokioChild>>>,
    /// PID when launched via service or UAC (we don't own the Child handle).
    external_pid: Arc<Mutex<Option<u32>>>,
    current_stage: Arc<Mutex<String>>,
    config_file: Arc<Mutex<Option<PathBuf>>>,
    auth_file: Arc<Mutex<Option<PathBuf>>>,
    mgmt_password_file: Arc<Mutex<Option<PathBuf>>>,
    stats: Arc<Mutex<VpnStats>>,
    management: Arc<tokio::sync::Mutex<Option<ManagementClient>>>,
    launch_mode: Arc<Mutex<LaunchMode>>,
}

impl OpenVpnManager {
    pub fn new(binary_path: String) -> Result<Self, OpenVpnError> {
        debug_log_to_file(&format!("[DEBUG] OpenVpnManager::new({})", binary_path));
        let path = PathBuf::from(&binary_path);

        if !path.exists() {
            return Err(OpenVpnError::BinaryNotFound(format!(
                "OpenVPN binary not found at: {}",
                path.display()
            )));
        }

        Ok(OpenVpnManager {
            binary_path: path,
            process: Arc::new(Mutex::new(None)),
            external_pid: Arc::new(Mutex::new(None)),
            current_stage: Arc::new(Mutex::new("disconnected".to_string())),
            config_file: Arc::new(Mutex::new(None)),
            auth_file: Arc::new(Mutex::new(None)),
            mgmt_password_file: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(VpnStats::default())),
            management: Arc::new(tokio::sync::Mutex::new(None)),
            launch_mode: Arc::new(Mutex::new(LaunchMode::Auto)),
        })
    }

    pub fn set_launch_mode(&self, mode: LaunchMode) {
        let mut m = self.launch_mode.lock().unwrap();
        *m = mode;
    }

    /// Launches the OpenVPN process with the provided configuration.
    pub async fn connect(
        &self,
        config: &str,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<(), OpenVpnError> {
        debug_log_to_file("[DEBUG] connect() called");

        // Gracefully terminate any previous connection.
        self.disconnect_internal(false).await;

        // Short delay for Windows to release the adapter.
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // --- Prepare temp files ------------------------------------------------
        let temp_dir = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let config_path = temp_dir.join(format!("openvpn_config_{}.ovpn", ts));
        std::fs::write(&config_path, config).map_err(OpenVpnError::IoError)?;

        let auth_path = if let (Some(u), Some(p)) = (&username, &password) {
            let ap = temp_dir.join(format!("openvpn_auth_{}.txt", ts));
            std::fs::write(&ap, format!("{}\n{}", u, p)).map_err(OpenVpnError::IoError)?;
            Some(ap)
        } else {
            None
        };

        // Store for cleanup
        *self.config_file.lock().unwrap() = Some(config_path.clone());
        if let Some(ref ap) = auth_path {
            *self.auth_file.lock().unwrap() = Some(ap.clone());
        }

        // --- Management interface preparation ----------------------------------
        let mgmt_port = find_free_port()?;
        let mgmt_password = generate_mgmt_password();

        // Write password file for UAC mode (stdin is unavailable with ShellExecuteEx).
        let mgmt_pw_path = temp_dir.join(format!("openvpn_mgmt_pw_{}.txt", ts));
        std::fs::write(&mgmt_pw_path, &mgmt_password).map_err(OpenVpnError::IoError)?;
        *self.mgmt_password_file.lock().unwrap() = Some(mgmt_pw_path.clone());

        // Record connection timestamp
        {
            let mut stats = self.stats.lock().unwrap();
            *stats = VpnStats::default();
            stats.connected_on = Some(std::time::SystemTime::now());
        }

        // Update stage
        {
            let mut s = self.current_stage.lock().unwrap();
            *s = "prepare".to_string();
        }

        // --- Determine launch mode and start process ---------------------------
        let mode = *self.launch_mode.lock().unwrap();
        let launched = match mode {
            LaunchMode::Service => {
                self.try_service_launch(&config_path, auth_path.as_ref(), mgmt_port, &mgmt_password).await
            }
            LaunchMode::Direct => {
                self.spawn_direct(&config_path, auth_path.as_ref(), mgmt_port, &mgmt_pw_path).await
            }
            LaunchMode::Uac => {
                self.spawn_elevated(&config_path, auth_path.as_ref(), mgmt_port, &mgmt_pw_path).await
            }
            LaunchMode::Auto => {
                // Try service first, then direct, then UAC.
                #[cfg(target_os = "windows")]
                {
                    if ServicePipeClient::is_service_available() {
                        match self.try_service_launch(&config_path, auth_path.as_ref(), mgmt_port, &mgmt_password).await {
                            Ok(()) => Ok(()),
                            Err(_) => {
                                debug_log_to_file("[DEBUG] Service launch failed, trying direct");
                                self.spawn_direct(&config_path, auth_path.as_ref(), mgmt_port, &mgmt_pw_path).await
                            }
                        }
                    } else {
                        self.spawn_direct(&config_path, auth_path.as_ref(), mgmt_port, &mgmt_pw_path).await
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    self.spawn_direct(&config_path, auth_path.as_ref(), mgmt_port, &mgmt_pw_path).await
                }
            }
        };

        launched?;

        // --- Connect management interface --------------------------------------
        let stage_arc = Arc::clone(&self.current_stage);
        let stats_arc = Arc::clone(&self.stats);

        let mgmt = ManagementClient::connect(mgmt_port, &mgmt_password, stage_arc, stats_arc)
            .await
            .map_err(|e| {
                debug_log_to_file(&format!("[DEBUG] Management connect failed: {:?}", e));
                e
            })?;

        *self.management.lock().await = Some(mgmt);
        debug_log_to_file("[DEBUG] Management interface connected, connect() done");

        Ok(())
    }

    /// Disconnect the VPN.
    pub async fn disconnect(&self) -> Result<(), OpenVpnError> {
        self.disconnect_internal(true).await;
        Ok(())
    }

    pub fn get_stage(&self) -> Result<String, OpenVpnError> {
        Ok(self.current_stage.lock().unwrap().clone())
    }

    pub fn get_stats(&self) -> Result<VpnStats, OpenVpnError> {
        Ok(self.stats.lock().unwrap().clone())
    }

    #[cfg(target_os = "windows")]
    pub fn is_service_available(&self) -> bool {
        ServicePipeClient::is_service_available()
    }

    #[cfg(not(target_os = "windows"))]
    pub fn is_service_available(&self) -> bool {
        false
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    async fn disconnect_internal(&self, reset_stage: bool) {
        debug_log_to_file("[DEBUG] disconnect_internal()");

        // 1. Try graceful SIGTERM via management interface.
        {
            let mut mgmt_guard = self.management.lock().await;
            if let Some(ref mgmt) = *mgmt_guard {
                let _ = mgmt.signal_terminate().await;
                // Give OpenVPN a moment to exit cleanly.
                tokio::time::sleep(Duration::from_millis(2000)).await;
            }
            *mgmt_guard = None;
        }

        // 2. Kill our tracked child process if still alive.
        {
            let mut proc = self.process.lock().unwrap();
            if let Some(mut child) = proc.take() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }

        // 3. For externally-launched processes (service/UAC), try taskkill by PID.
        {
            let mut ext = self.external_pid.lock().unwrap();
            if let Some(pid) = ext.take() {
                let _ = kill_process_by_pid(pid).await;
            }
        }

        // Short delay for adapter release.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 4. Clean up temp files.
        for file_lock in [&self.config_file, &self.auth_file, &self.mgmt_password_file] {
            let mut guard = file_lock.lock().unwrap();
            if let Some(ref p) = *guard {
                let _ = std::fs::remove_file(p);
            }
            *guard = None;
        }

        if reset_stage {
            *self.current_stage.lock().unwrap() = "disconnected".to_string();
            *self.stats.lock().unwrap() = VpnStats::default();
        }
    }

    /// Build the common OpenVPN command-line options string.
    fn build_options(
        config_path: &PathBuf,
        auth_path: Option<&PathBuf>,
        mgmt_port: u16,
        mgmt_pw_file: &PathBuf,
    ) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        args.push("--config".into());
        args.push(config_path.to_string_lossy().into_owned());
        if let Some(ap) = auth_path {
            args.push("--auth-user-pass".into());
            args.push(ap.to_string_lossy().into_owned());
        }
        args.push("--verb".into());
        args.push("4".into());
        args.push("--windows-driver".into());
        args.push("wintun".into());
        args.push("--management".into());
        args.push("127.0.0.1".into());
        args.push(mgmt_port.to_string());
        // Use password file instead of stdin for robustness across all launch modes.
        args.push("--management-password-file".into());
        args.push(mgmt_pw_file.to_string_lossy().into_owned());
        args
    }

    /// Build a single-string option line for the service pipe protocol.
    fn build_options_string(
        config_filename: &str,
        auth_path: Option<&PathBuf>,
        mgmt_port: u16,
        mgmt_pw_file: &PathBuf,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(format!("--config \"{}\"", config_filename));
        if let Some(ap) = auth_path {
            parts.push(format!("--auth-user-pass \"{}\"", ap.to_string_lossy()));
        }
        parts.push("--verb 4".into());
        parts.push("--windows-driver wintun".into());
        parts.push(format!("--management 127.0.0.1 {}", mgmt_port));
        parts.push(format!(
            "--management-password-file \"{}\"",
            mgmt_pw_file.to_string_lossy()
        ));
        parts.push("--pull-filter ignore route-method".into());
        parts.join(" ")
    }

    // --- Launch mode: Interactive Service ---

    async fn try_service_launch(
        &self,
        config_path: &PathBuf,
        auth_path: Option<&PathBuf>,
        mgmt_port: u16,
        mgmt_password: &str,
    ) -> Result<(), OpenVpnError> {
        #[cfg(target_os = "windows")]
        {
            debug_log_to_file("[DEBUG] Attempting service launch");
            let config_dir = config_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_string_lossy()
                .into_owned();

            let config_filename = config_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();

            let mgmt_pw_path = self.mgmt_password_file.lock().unwrap().clone()
                .ok_or_else(|| OpenVpnError::ConfigurationError("No mgmt password file".into()))?;

            let options = Self::build_options_string(
                &config_filename,
                auth_path,
                mgmt_port,
                &mgmt_pw_path,
            );

            let pid = ServicePipeClient::start_openvpn(&config_dir, &options, mgmt_password)?;
            debug_log_to_file(&format!("[DEBUG] Service started openvpn PID: {}", pid));
            *self.external_pid.lock().unwrap() = Some(pid);
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (config_path, auth_path, mgmt_port, mgmt_password);
            Err(OpenVpnError::ServiceError(
                "Interactive Service only available on Windows".into(),
            ))
        }
    }

    // --- Launch mode: Direct spawn ---

    async fn spawn_direct(
        &self,
        config_path: &PathBuf,
        auth_path: Option<&PathBuf>,
        mgmt_port: u16,
        mgmt_pw_file: &PathBuf,
    ) -> Result<(), OpenVpnError> {
        debug_log_to_file("[DEBUG] spawn_direct()");
        let args = Self::build_options(config_path, auth_path, mgmt_port, mgmt_pw_file);

        let mut cmd = TokioCommand::new(&self.binary_path);
        for arg in &args {
            cmd.arg(arg);
        }
        cmd.stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null());

        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd.spawn().map_err(|e| {
            OpenVpnError::ProcessExecutionFailed(format!("Failed to spawn openvpn: {}", e))
        })?;

        debug_log_to_file(&format!("[DEBUG] Direct spawn PID: {:?}", child.id()));
        *self.process.lock().unwrap() = Some(child);
        Ok(())
    }

    // --- Launch mode: UAC elevation ---

    async fn spawn_elevated(
        &self,
        config_path: &PathBuf,
        auth_path: Option<&PathBuf>,
        mgmt_port: u16,
        mgmt_pw_file: &PathBuf,
    ) -> Result<(), OpenVpnError> {
        #[cfg(target_os = "windows")]
        {
            debug_log_to_file("[DEBUG] spawn_elevated() via ShellExecuteEx");
            let args = Self::build_options(config_path, auth_path, mgmt_port, mgmt_pw_file);
            let params = args.join(" ");

            let binary_str: Vec<u16> = self
                .binary_path
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let params_str: Vec<u16> = params
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let verb: Vec<u16> = "runas\0".encode_utf16().collect();

            use windows_sys::Win32::UI::Shell::ShellExecuteExW;
            use windows_sys::Win32::UI::Shell::SHELLEXECUTEINFOW;

            let mut sei = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: 0x00000040, // SEE_MASK_NOCLOSEPROCESS
                hwnd: std::ptr::null_mut(),
                lpVerb: verb.as_ptr(),
                lpFile: binary_str.as_ptr(),
                lpParameters: params_str.as_ptr(),
                lpDirectory: std::ptr::null(),
                nShow: 0, // SW_HIDE
                hInstApp: std::ptr::null_mut(),
                lpIDList: std::ptr::null_mut(),
                lpClass: std::ptr::null(),
                hkeyClass: std::ptr::null_mut(),
                dwHotKey: 0,
                Anonymous: unsafe { std::mem::zeroed() },
                hProcess: std::ptr::null_mut(),
            };

            let ok = unsafe { ShellExecuteExW(&mut sei) };
            if ok == 0 {
                return Err(OpenVpnError::ElevationRequired(
                    "User cancelled UAC dialog or elevation failed".into(),
                ));
            }

            // Extract PID from the process handle.
            if !sei.hProcess.is_null() {
                let pid =
                    unsafe { windows_sys::Win32::System::Threading::GetProcessId(sei.hProcess) };
                debug_log_to_file(&format!("[DEBUG] Elevated PID: {}", pid));
                *self.external_pid.lock().unwrap() = Some(pid);
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(sei.hProcess);
                }
            }

            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (config_path, auth_path, mgmt_port, mgmt_pw_file);
            Err(OpenVpnError::ElevationRequired(
                "UAC elevation only available on Windows".into(),
            ))
        }
    }
}

/// Kill a specific process by PID.
async fn kill_process_by_pid(pid: u32) -> Result<(), OpenVpnError> {
    debug_log_to_file(&format!("[DEBUG] Killing PID {}", pid));
    let mut cmd = TokioCommand::new("taskkill");
    cmd.args(["/F", "/PID", &pid.to_string(), "/T"]);
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd.output().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(())
}
