//! OpenVPN process management on Windows

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tokio::process::{Child as TokioChild, Command as TokioCommand};
use tokio::io::{AsyncBufReadExt, BufReader};
use crate::error::OpenVpnError;

// Helper function to kill all OpenVPN processes on Windows
async fn kill_all_openvpn_processes() {
    debug_log_to_file("[DEBUG] Attempting to kill all OpenVPN processes...");
    
    // Use taskkill to forcefully kill all openvpn.exe processes
    let mut kill_cmd = TokioCommand::new("taskkill");
    kill_cmd.args(&["/F", "/IM", "openvpn.exe", "/T"]);
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        kill_cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = kill_cmd.output().await;
    
    match output {
        Ok(output) => {
            if output.status.success() {
                debug_log_to_file("[DEBUG] Successfully killed all OpenVPN processes");
                let stdout = String::from_utf8_lossy(&output.stdout);
                if !stdout.is_empty() {
                    debug_log_to_file(&format!("[DEBUG] taskkill output: {}", stdout));
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // It's OK if no processes were found
                if stderr.contains("not found") || stderr.contains("not running") {
                    debug_log_to_file("[DEBUG] No OpenVPN processes found to kill");
                } else {
                    debug_log_to_file(&format!("[DEBUG] Warning: taskkill failed: {}", stderr));
                }
            }
        }
        Err(e) => {
            debug_log_to_file(&format!("[DEBUG] Warning: Failed to execute taskkill: {}", e));
        }
    }
    
    // Wait a bit for Windows to release resources
    tokio::time::sleep(Duration::from_millis(1000)).await;
}

// Set to true to enable debug logging
const ENABLE_DEBUG_LOGS: bool = false;

// Helper function to write debug logs to a file
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
    // Also print to stderr (visible in debugger)
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

/// Main OpenVPN manager
pub struct OpenVpnManager {
    binary_path: PathBuf,
    process: Arc<Mutex<Option<TokioChild>>>,
    current_stage: Arc<Mutex<String>>,
    config_file: Arc<Mutex<Option<PathBuf>>>,  // Keep config file during execution
    auth_file: Arc<Mutex<Option<PathBuf>>>,   // Keep auth file during execution
    status_file: Arc<Mutex<Option<PathBuf>>>,  // Keep status file path for statistics
    stats: Arc<Mutex<VpnStats>>,  // Connection statistics
}

impl OpenVpnManager {
    /// Creates a new OpenVPN manager
    pub fn new(binary_path: String) -> Result<Self, OpenVpnError> {
        let msg = format!("[DEBUG] OpenVpnManager::new() called with binary_path: {}", binary_path);
        debug_log_to_file(&msg);
        let path = PathBuf::from(binary_path.clone());
        
        if !path.exists() {
            let msg = format!("[DEBUG] ERROR: Binary not found at: {}", path.display());
            debug_log_to_file(&msg);
            return Err(OpenVpnError::BinaryNotFound(
                format!("OpenVPN binary not found at: {}", path.display())
            ));
        }

        let msg = format!("[DEBUG] Binary exists at: {}", path.display());
        debug_log_to_file(&msg);
        debug_log_to_file("[DEBUG] OpenVpnManager created successfully");

        Ok(OpenVpnManager {
            binary_path: path,
            process: Arc::new(Mutex::new(None)),
            current_stage: Arc::new(Mutex::new("disconnected".to_string())),
            config_file: Arc::new(Mutex::new(None)),
            auth_file: Arc::new(Mutex::new(None)),
            status_file: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(VpnStats::default())),
        })
    }

    /// Launches the OpenVPN process with the provided configuration
    pub async fn connect(
        &self,
        config: &str,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<(), OpenVpnError> {
        debug_log_to_file("[DEBUG] OpenVpnManager::connect() called");
        debug_log_to_file(&format!("[DEBUG] Config length: {} bytes", config.len()));
        debug_log_to_file(&format!("[DEBUG] Username provided: {}", username.is_some()));
        debug_log_to_file(&format!("[DEBUG] Password provided: {}", password.is_some()));
        
        // Kill ALL OpenVPN processes (including daemons) before starting a new connection
        debug_log_to_file("[DEBUG] Killing all OpenVPN processes before new connection");
        kill_all_openvpn_processes().await;
        
        // Also clean up our tracked process if any
        {
            let mut process = self.process.lock().unwrap();
            if let Some(mut child) = process.take() {
                debug_log_to_file("[DEBUG] Cleaning up tracked OpenVPN process");
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
        
        // Wait a bit more for Windows to release the previous tunnel adapter instance.
        debug_log_to_file("[DEBUG] Waiting for tunnel adapter to be released...");
        tokio::time::sleep(Duration::from_millis(3000)).await;
        debug_log_to_file("[DEBUG] Tunnel adapter release wait completed");

        // Create a temporary file for the configuration
        // Create in the system temporary directory
        let temp_dir = std::env::temp_dir();
        debug_log_to_file(&format!("[DEBUG] Temp directory: {}", temp_dir.display()));
        
        let config_file = temp_dir.join(format!("openvpn_config_{}.ovpn", 
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()));
        
        debug_log_to_file(&format!("[DEBUG] Writing config file to: {}", config_file.display()));
        std::fs::write(&config_file, config)
            .map_err(|e| {
                debug_log_to_file(&format!("[DEBUG] ERROR: Failed to write config file: {}", e));
                OpenVpnError::IoError(e)
            })?;
        let config_size = std::fs::metadata(&config_file).map(|m| m.len()).unwrap_or(0);
        debug_log_to_file(&format!("[DEBUG] Config file written successfully ({} bytes)", config_size));

        // Create a temporary file for credentials if necessary
        let auth_file = if username.is_some() && password.is_some() {
            let auth_content = format!(
                "{}\n{}",
                username.as_ref().unwrap(),
                password.as_ref().unwrap()
            );
            let auth_path = temp_dir.join(format!("openvpn_auth_{}.txt",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()));
            debug_log_to_file(&format!("[DEBUG] Writing auth file to: {}", auth_path.display()));
            std::fs::write(&auth_path, auth_content)
                .map_err(|e| {
                    debug_log_to_file(&format!("[DEBUG] ERROR: Failed to write auth file: {}", e));
                    OpenVpnError::IoError(e)
                })?;
            debug_log_to_file("[DEBUG] Auth file written successfully");
            Some(auth_path)
        } else {
            debug_log_to_file("[DEBUG] No auth file needed (no username/password)");
            None
        };
        
        // Store file paths for later cleanup
        {
            let mut stored_config = self.config_file.lock().unwrap();
            *stored_config = Some(config_file.clone());
        }
        
        if let Some(ref auth_path) = auth_file {
            let mut stored_auth = self.auth_file.lock().unwrap();
            *stored_auth = Some(auth_path.clone());
        }

        // Build the OpenVPN command
        debug_log_to_file("[DEBUG] Building OpenVPN command");
        debug_log_to_file(&format!("[DEBUG] Binary path: {}", self.binary_path.display()));
        let mut cmd = TokioCommand::new(&self.binary_path);
        cmd.arg("--config").arg(&config_file);
        debug_log_to_file(&format!("[DEBUG] Command arg: --config {}", config_file.display()));
        
        if let Some(auth_path) = &auth_file {
            cmd.arg("--auth-user-pass").arg(auth_path);
            debug_log_to_file(&format!("[DEBUG] Command arg: --auth-user-pass {}", auth_path.display()));
        }
        
        // Add --verb 4 for verbose logging
        cmd.arg("--verb").arg("4");
        debug_log_to_file("[DEBUG] Command arg: --verb 4");

        // Force OpenVPN to use Wintun on Windows to avoid TAP installer issues.
        cmd.arg("--windows-driver").arg("wintun");
        debug_log_to_file("[DEBUG] Command arg: --windows-driver wintun");
        
        // Add --status option to get statistics
        // OpenVPN will write statistics to a file every 2 seconds
        let status_file = temp_dir.join(format!("openvpn_status_{}.txt",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()));
        cmd.arg("--status").arg(&status_file).arg("2");
        debug_log_to_file(&format!("[DEBUG] Command arg: --status {} 2", status_file.display()));
        
        // Store status file path for later cleanup and reading
        {
            let mut stored_status = self.status_file.lock().unwrap();
            *stored_status = Some(status_file.clone());
        }

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        
        // Prevent OpenVPN from opening a visible console window on Windows.
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        
        debug_log_to_file("[DEBUG] Command prepared, spawning process...");
        
        // Log the full command for debugging
        let cmd_str = format!("{:?}", cmd);
        debug_log_to_file(&format!("[DEBUG] Full command: {}", cmd_str));

        // Launch the process
        let mut child = cmd.spawn()
            .map_err(|e| {
                debug_log_to_file(&format!("[DEBUG] ERROR: Failed to spawn OpenVPN process: {}", e));
                OpenVpnError::ProcessExecutionFailed(
                    format!("Failed to spawn OpenVPN process: {}", e)
                )
            })?;
        
        let pid = child.id();
        debug_log_to_file(&format!("[DEBUG] Process spawned successfully, PID: {:?}", pid));

        // Read stdout and stderr in parallel to detect stages
        debug_log_to_file("[DEBUG] Taking stdout and stderr handles");
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        
        let process_arc = Arc::clone(&self.process);
        let stage_arc = Arc::clone(&self.current_stage);
        let stats_arc = Arc::clone(&self.stats);
        let status_file_arc = Arc::clone(&self.status_file);
        
        // Record connection timestamp
        {
            let mut stats = stats_arc.lock().unwrap();
            stats.connected_on = Some(std::time::SystemTime::now());
        }
        debug_log_to_file("[DEBUG] Connection timestamp recorded");
        
        // Task to read status file periodically for statistics
        let stats_status = Arc::clone(&stats_arc);
        let status_file_status = Arc::clone(&status_file_arc);
        debug_log_to_file("[DEBUG] Spawning status file reader task");
        tokio::spawn(async move {
            debug_log_to_file("[DEBUG] Status file reader task started");
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                
                // Get the status file path
                let status_file_path = {
                    let status_file_guard = status_file_status.lock().unwrap();
                    status_file_guard.clone()
                };
                
                if let Some(ref status_path) = status_file_path {
                    if let Ok(content) = std::fs::read_to_string(status_path) {
                        parse_status_file(&content, &stats_status);
                    }
                } else {
                    // Status file no longer available, exit task
                    debug_log_to_file("[DEBUG] Status file no longer available, exiting reader task");
                    break;
                }
            }
            debug_log_to_file("[DEBUG] Status file reader task ended");
        });

        // Task to read stdout
        let stats_stdout = Arc::clone(&stats_arc);
        let stage_stdout = Arc::clone(&stage_arc);
        debug_log_to_file("[DEBUG] Spawning stdout reader task");
        tokio::spawn(async move {
            debug_log_to_file("[DEBUG] stdout reader task started");
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut line_count = 0u64;
            
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        line_count += 1;
                        // Only log lines that contain important information or stage changes
                        // Don't log every single line to reduce verbosity
                        
                        // Parse lines to detect stages
                        let stage = parse_stage_from_output(&line);
                        if let Some(new_stage) = stage {
                            let mut current = stage_stdout.lock().unwrap();
                            let old_stage = current.clone();
                            
                            // Only log if the stage actually changed
                            if old_stage != new_stage {
                                debug_log_to_file(&format!("[DEBUG] stdout: Stage updated from '{}' to '{}'", old_stage, new_stage));
                                *current = new_stage.clone();
                                
                                // If connected, update the timestamp
                                if new_stage == "connected" {
                                    debug_log_to_file("[DEBUG] stdout: CONNECTED stage detected!");
                                    let mut stats = stats_stdout.lock().unwrap();
                                    if stats.connected_on.is_none() {
                                        stats.connected_on = Some(std::time::SystemTime::now());
                                        debug_log_to_file("[DEBUG] stdout: Connection timestamp updated");
                                    }
                                }
                            } else {
                                // Stage didn't change, don't log
                                drop(current);
                            }
                        }
                        // Only log lines with errors, warnings, or critical messages
                        // Remove "TAP" and "route" from the filter as they generate too many logs
                        else if line.contains("ERROR") || line.contains("WARNING") || 
                                line.contains("FATAL") || line.contains("Exiting") ||
                                line.contains("Initialization Sequence Completed") ||
                                line.contains("fatal error") || line.contains("failed") {
                            debug_log_to_file(&format!("[DEBUG] stdout line #{}: {}", line_count, line));
                        }
                        // Don't log "No stage detected" messages at all
                        
                        // Parse statistics from output
                        parse_stats_from_output(&line, &stats_stdout);
                    }
                    Ok(None) => {
                        debug_log_to_file(&format!("[DEBUG] stdout: EOF reached (total lines: {})", line_count));
                        break;
                    }
                    Err(e) => {
                        debug_log_to_file(&format!("[DEBUG] stdout: ERROR reading line: {} (total lines read: {})", e, line_count));
                        break;
                    }
                }
            }
            debug_log_to_file("[DEBUG] stdout reader task ended");
        });

        // Task to read stderr
        let stats_stderr = Arc::clone(&stats_arc);
        let stage_stderr = Arc::clone(&stage_arc);
        debug_log_to_file("[DEBUG] Spawning stderr reader task");
        tokio::spawn(async move {
            debug_log_to_file("[DEBUG] stderr reader task started");
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            let mut line_count = 0u64;
            
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        line_count += 1;
                        debug_log_to_file(&format!("[DEBUG] stderr line #{}: {}", line_count, line));
                        
                        // Parse lines to detect stages
                        let stage = parse_stage_from_output(&line);
                        if let Some(new_stage) = stage {
                            debug_log_to_file(&format!("[DEBUG] stderr: Stage detected: {}", new_stage));
                            let mut current = stage_stderr.lock().unwrap();
                            let old_stage = current.clone();
                            *current = new_stage.clone();
                            debug_log_to_file(&format!("[DEBUG] stderr: Stage updated from '{}' to '{}'", old_stage, new_stage));
                            
                            // If connected, update the timestamp
                            if new_stage == "connected" {
                                debug_log_to_file("[DEBUG] stderr: CONNECTED stage detected!");
                                let mut stats = stats_stderr.lock().unwrap();
                                if stats.connected_on.is_none() {
                                    stats.connected_on = Some(std::time::SystemTime::now());
                                    debug_log_to_file("[DEBUG] stderr: Connection timestamp updated");
                                }
                            }
                        } else {
                            debug_log_to_file("[DEBUG] stderr: No stage detected in line");
                        }
                        
                        // Parse statistics from output
                        parse_stats_from_output(&line, &stats_stderr);
                    }
                    Ok(None) => {
                        debug_log_to_file(&format!("[DEBUG] stderr: EOF reached (total lines: {})", line_count));
                        break;
                    }
                    Err(e) => {
                        debug_log_to_file(&format!("[DEBUG] stderr: ERROR reading line: {} (total lines read: {})", e, line_count));
                        break;
                    }
                }
            }
            debug_log_to_file("[DEBUG] stderr reader task ended");
        });

        // Store the process
        {
            let mut process = process_arc.lock().unwrap();
            *process = Some(child);
        }
        debug_log_to_file("[DEBUG] Process stored, connect() returning Ok");

        Ok(())
    }

    /// Disconnects the VPN
    pub async fn disconnect(&self) -> Result<(), OpenVpnError> {
        debug_log_to_file("[DEBUG] OpenVpnManager::disconnect() called");
        
        // Kill ALL OpenVPN processes (including daemons) when disconnecting
        debug_log_to_file("[DEBUG] Killing all OpenVPN processes during disconnect");
        kill_all_openvpn_processes().await;
        
        // Also clean up our tracked process if any
        let mut process = self.process.lock().unwrap();
        if let Some(mut child) = process.take() {
            debug_log_to_file("[DEBUG] Killing tracked OpenVPN process");
            // Try to kill the process, but don't fail if it's already dead
            match child.kill().await {
                Ok(_) => {
                    debug_log_to_file("[DEBUG] OpenVPN process killed successfully");
                }
                Err(e) => {
                    debug_log_to_file(&format!("[DEBUG] Warning: Failed to kill process (may already be dead): {}", e));
                }
            }
            
            // Wait for the process to finish
            let _ = child.wait().await;
            debug_log_to_file("[DEBUG] OpenVPN process cleanup completed");
        } else {
            debug_log_to_file("[DEBUG] No tracked OpenVPN process to disconnect");
        }
        
        // Wait for Windows to release the previous tunnel adapter instance.
        debug_log_to_file("[DEBUG] Waiting for tunnel adapter to be released...");
        tokio::time::sleep(Duration::from_millis(1500)).await;
        debug_log_to_file("[DEBUG] Tunnel adapter release wait completed");

        // Clean up temporary files
        {
            let mut config_file = self.config_file.lock().unwrap();
            if let Some(ref path) = *config_file {
                let _ = std::fs::remove_file(path);
            }
            *config_file = None;
        }
        
        {
            let mut auth_file = self.auth_file.lock().unwrap();
            if let Some(ref path) = *auth_file {
                let _ = std::fs::remove_file(path);
            }
            *auth_file = None;
        }
        
        {
            let mut status_file = self.status_file.lock().unwrap();
            if let Some(ref path) = *status_file {
                let _ = std::fs::remove_file(path);
            }
            *status_file = None;
        }

        // Reset the stage
        let mut stage = self.current_stage.lock().unwrap();
        *stage = "disconnected".to_string();
        
        // Reset statistics
        let mut stats = self.stats.lock().unwrap();
        *stats = VpnStats::default();

        Ok(())
    }

    /// Gets the current stage
    pub fn get_stage(&self) -> Result<String, OpenVpnError> {
        let stage = self.current_stage.lock().unwrap();
        let stage_str = stage.clone();
        debug_log_to_file(&format!("[DEBUG] get_stage() called, returning: {}", stage_str));
        Ok(stage_str)
    }
    
    /// Gets the current statistics
    pub fn get_stats(&self) -> Result<VpnStats, OpenVpnError> {
        let stats = self.stats.lock().unwrap();
        Ok(stats.clone())
    }
}

/// Parses OpenVPN output to detect the stage
fn parse_stage_from_output(line: &str) -> Option<String> {
    let line_lower = line.to_lowercase();
    
    // Mapping of OpenVPN messages to stages
    if line_lower.contains("initialization sequence completed") {
        debug_log_to_file("[DEBUG] parse_stage: Matched 'initialization sequence completed' -> 'connected'");
        Some("connected".to_string())
    } else if line_lower.contains("auth_failed")
        || line_lower.contains("authentication failed")
        || line_lower.contains("tls error")
        || line_lower.contains("tls handshake failed")
        || line_lower.contains("cannot resolve host address")
        || line_lower.contains("resolve error")
        || line_lower.contains("network is unreachable")
        || line_lower.contains("connection timed out")
        || line_lower.contains("connection reset")
        || line_lower.contains("connection refused")
        || line_lower.contains("options error")
        || line_lower.contains("fatal")
        || line_lower.contains("certificate verify failed")
        || line_lower.contains("verify error") {
        debug_log_to_file(&format!("[DEBUG] parse_stage: Critical VPN error detected: {} -> 'error'", line));
        Some("error".to_string())
    } else if line_lower.contains("connecting") || line_lower.contains("waiting") {
        debug_log_to_file("[DEBUG] parse_stage: Matched 'connecting/waiting' -> 'connecting'");
        Some("connecting".to_string())
    } else if line_lower.contains("disconnecting") || line_lower.contains("exiting") {
        debug_log_to_file("[DEBUG] parse_stage: Matched 'disconnecting/exiting' -> 'disconnecting'");
        Some("disconnecting".to_string())
    } else if line_lower.contains("authentication") || line_lower.contains("auth") {
        debug_log_to_file("[DEBUG] parse_stage: Matched 'authentication/auth' -> 'authenticating'");
        Some("authenticating".to_string())
    } else if line_lower.contains("preserving previous tun/tap instance") {
        // This line can appear during normal adapter reuse.
        None
    } else if (line_lower.contains("wintun") || line_lower.contains("tun/tap"))
        && (line_lower.contains("cannot create wintun adapter")
            || line_lower.contains("wintun.dll")
            || line_lower.contains("cannot allocate tun/tap")
            || line_lower.contains("error_gen_failure")) {
        // Specific Wintun/TUN errors - these are critical.
        debug_log_to_file(&format!("[DEBUG] parse_stage: Wintun/TUN error detected: {} -> 'error'", line));
        Some("error".to_string())
    } else if line_lower.contains("error") || line_lower.contains("failed") {
        debug_log_to_file("[DEBUG] parse_stage: Matched 'error/failed' -> 'error'");
        Some("error".to_string())
    } else {
        None
    }
}

/// Parses statistics from OpenVPN output
/// 
/// OpenVPN can display statistics in different formats.
/// We look for patterns like:
/// - "TCP/UDP read bytes" / "TCP/UDP write bytes"
/// - "TUN/TAP read bytes" / "TUN/TAP write bytes"
/// - Management interface statistics messages
/// 
/// NOTE: OpenVPN doesn't always display statistics in stdout/stderr by default.
/// For accurate statistics, the --management option with a socket should be used.
/// For now, we use basic parsing that may not capture all statistics.
fn parse_stats_from_output(line: &str, stats: &Arc<Mutex<VpnStats>>) {
    let line_lower = line.to_lowercase();
    
    // Patterns to detect statistics in OpenVPN output
    // Possible format: "TCP/UDP read bytes, [number]"
    // Possible format: "TUN/TAP read bytes, [number]"
    // Possible format: "read bytes, [number]"
    // Possible format: "write bytes, [number]"
    
    // Parsing for bytes read (download)
    if line_lower.contains("read bytes") || line_lower.contains("bytes read") {
        // Try to extract the number
        if let Some(bytes) = extract_number_after_keyword(&line_lower, "read bytes") {
            let mut stats_guard = stats.lock().unwrap();
            // Use absolute value (no increment, as it's a total)
            stats_guard.byte_in = bytes;
        } else if let Some(bytes) = extract_number_after_keyword(&line_lower, "bytes read") {
            let mut stats_guard = stats.lock().unwrap();
            stats_guard.byte_in = bytes;
        }
    }
    
    // Parsing for bytes written (upload)
    if line_lower.contains("write bytes") || line_lower.contains("bytes write") {
        // Try to extract the number
        if let Some(bytes) = extract_number_after_keyword(&line_lower, "write bytes") {
            let mut stats_guard = stats.lock().unwrap();
            stats_guard.byte_out = bytes;
        } else if let Some(bytes) = extract_number_after_keyword(&line_lower, "bytes write") {
            let mut stats_guard = stats.lock().unwrap();
            stats_guard.byte_out = bytes;
        }
    }
    
    // Parsing for packets (rarer in standard output)
    if line_lower.contains("packets read") || line_lower.contains("read packets") {
        if let Some(packets) = extract_number_after_keyword(&line_lower, "packets read") {
            let mut stats_guard = stats.lock().unwrap();
            stats_guard.packets_in = packets;
        } else if let Some(packets) = extract_number_after_keyword(&line_lower, "read packets") {
            let mut stats_guard = stats.lock().unwrap();
            stats_guard.packets_in = packets;
        }
    }
    
    if line_lower.contains("packets write") || line_lower.contains("write packets") {
        if let Some(packets) = extract_number_after_keyword(&line_lower, "packets write") {
            let mut stats_guard = stats.lock().unwrap();
            stats_guard.packets_out = packets;
        } else if let Some(packets) = extract_number_after_keyword(&line_lower, "write packets") {
            let mut stats_guard = stats.lock().unwrap();
            stats_guard.packets_out = packets;
        }
    }
    
    // Note: For more accurate and real-time statistics,
    // we should implement support for OpenVPN's --management interface
    // which allows querying statistics via a TCP socket
}

/// Parses OpenVPN status file to extract statistics
/// 
/// The status file format is:
/// - Line 1: "OpenVPN STATISTICS"
/// - Line 2: "Updated,<timestamp>"
/// - Line 3+: Various statistics lines like "TUN/TAP read bytes,<bytes>"
fn parse_status_file(content: &str, stats: &Arc<Mutex<VpnStats>>) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("OpenVPN STATISTICS") || line.starts_with("Updated,") {
            continue;
        }
        
        // Parse lines like "TUN/TAP read bytes,12345"
        if line.starts_with("TUN/TAP read bytes,") {
            if let Some(bytes_str) = line.split(',').nth(1) {
                if let Ok(bytes) = bytes_str.trim().parse::<u64>() {
                    let mut stats_guard = stats.lock().unwrap();
                    stats_guard.byte_in = bytes;
                }
            }
        } else if line.starts_with("TUN/TAP write bytes,") {
            if let Some(bytes_str) = line.split(',').nth(1) {
                if let Ok(bytes) = bytes_str.trim().parse::<u64>() {
                    let mut stats_guard = stats.lock().unwrap();
                    stats_guard.byte_out = bytes;
                }
            }
        } else if line.starts_with("TCP/UDP read bytes,") {
            if let Some(bytes_str) = line.split(',').nth(1) {
                if let Ok(bytes) = bytes_str.trim().parse::<u64>() {
                    let mut stats_guard = stats.lock().unwrap();
                    stats_guard.byte_in = bytes;
                }
            }
        } else if line.starts_with("TCP/UDP write bytes,") {
            if let Some(bytes_str) = line.split(',').nth(1) {
                if let Ok(bytes) = bytes_str.trim().parse::<u64>() {
                    let mut stats_guard = stats.lock().unwrap();
                    stats_guard.byte_out = bytes;
                }
            }
        }
    }
}

/// Extracts a number after a keyword in a line
/// 
/// Searches for the keyword and extracts the first number that follows (may be separated by spaces/punctuation)
fn extract_number_after_keyword(line: &str, keyword: &str) -> Option<u64> {
    if let Some(pos) = line.find(keyword) {
        let after_keyword = &line[pos + keyword.len()..];
        // Find the first number in the string (may be preceded by punctuation)
        let number_str: String = after_keyword
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit() || *c == ',')
            .filter(|c| c.is_ascii_digit())
            .collect();
        
        if !number_str.is_empty() {
            number_str.parse::<u64>().ok()
        } else {
            None
        }
    } else {
        None
    }
}

