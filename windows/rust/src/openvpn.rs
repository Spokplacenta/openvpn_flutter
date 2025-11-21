//! OpenVPN process management on Windows

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::process::{Child as TokioChild, Command as TokioCommand};
use tokio::io::{AsyncBufReadExt, BufReader};
use crate::error::OpenVpnError;

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
    stats: Arc<Mutex<VpnStats>>,  // Connection statistics
}

impl OpenVpnManager {
    /// Creates a new OpenVPN manager
    pub fn new(binary_path: String) -> Result<Self, OpenVpnError> {
        let path = PathBuf::from(binary_path);
        
        if !path.exists() {
            return Err(OpenVpnError::BinaryNotFound(
                format!("OpenVPN binary not found at: {}", path.display())
            ));
        }

        Ok(OpenVpnManager {
            binary_path: path,
            process: Arc::new(Mutex::new(None)),
            current_stage: Arc::new(Mutex::new("disconnected".to_string())),
            config_file: Arc::new(Mutex::new(None)),
            auth_file: Arc::new(Mutex::new(None)),
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
        // Check that no process is already running
        {
            let process = self.process.lock().unwrap();
            if process.is_some() {
                return Err(OpenVpnError::ConnectionFailed(
                    "A connection is already in progress".to_string()
                ));
            }
        }

        // Create a temporary file for the configuration
        // Create in the system temporary directory
        let temp_dir = std::env::temp_dir();
        let config_file = temp_dir.join(format!("openvpn_config_{}.ovpn", 
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()));
        
        std::fs::write(&config_file, config)
            .map_err(|e| OpenVpnError::IoError(e))?;

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
            std::fs::write(&auth_path, auth_content)
                .map_err(|e| OpenVpnError::IoError(e))?;
            Some(auth_path)
        } else {
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
        let mut cmd = TokioCommand::new(&self.binary_path);
        cmd.arg("--config").arg(&config_file);
        
        if let Some(auth_path) = &auth_file {
            cmd.arg("--auth-user-pass").arg(auth_path);
        }
        
        // Add --management option to get statistics
        // Note: For now, we don't use the management socket
        // but we could implement it later for more accurate statistics
        // cmd.arg("--management").arg("127.0.0.1").arg("7505");

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Launch the process
        let mut child = cmd.spawn()
            .map_err(|e| OpenVpnError::ProcessExecutionFailed(
                format!("Failed to spawn OpenVPN process: {}", e)
            ))?;

        // Read stdout and stderr in parallel to detect stages
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        
        let process_arc = Arc::clone(&self.process);
        let stage_arc = Arc::clone(&self.current_stage);
        let stats_arc = Arc::clone(&self.stats);
        
        // Record connection timestamp
        {
            let mut stats = stats_arc.lock().unwrap();
            stats.connected_on = Some(std::time::SystemTime::now());
        }

        // Task to read stdout
        let stats_stdout = Arc::clone(&stats_arc);
        let stage_stdout = Arc::clone(&stage_arc);
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            
            while let Ok(Some(line)) = lines.next_line().await {
                // Parse lines to detect stages
                let stage = parse_stage_from_output(&line);
                if let Some(new_stage) = stage {
                    let mut current = stage_stdout.lock().unwrap();
                    *current = new_stage.clone();
                    
                    // If connected, update the timestamp
                    if new_stage == "connected" {
                        let mut stats = stats_stdout.lock().unwrap();
                        if stats.connected_on.is_none() {
                            stats.connected_on = Some(std::time::SystemTime::now());
                        }
                    }
                }
                
                // Parse statistics from output
                parse_stats_from_output(&line, &stats_stdout);
            }
        });

        // Task to read stderr
        let stats_stderr = Arc::clone(&stats_arc);
        let stage_stderr = Arc::clone(&stage_arc);
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            
            while let Ok(Some(line)) = lines.next_line().await {
                // Parse lines to detect stages
                let stage = parse_stage_from_output(&line);
                if let Some(new_stage) = stage {
                    let mut current = stage_stderr.lock().unwrap();
                    *current = new_stage.clone();
                    
                    // If connected, update the timestamp
                    if new_stage == "connected" {
                        let mut stats = stats_stderr.lock().unwrap();
                        if stats.connected_on.is_none() {
                            stats.connected_on = Some(std::time::SystemTime::now());
                        }
                    }
                }
                
                // Parse statistics from output
                parse_stats_from_output(&line, &stats_stderr);
            }
        });

        // Store the process
        {
            let mut process = process_arc.lock().unwrap();
            *process = Some(child);
        }

        Ok(())
    }

    /// Disconnects the VPN
    pub async fn disconnect(&self) -> Result<(), OpenVpnError> {
        let mut process = self.process.lock().unwrap();
        
        if let Some(mut child) = process.take() {
            child.kill()
                .await
                .map_err(|e| OpenVpnError::ProcessExecutionFailed(
                    format!("Failed to kill OpenVPN process: {}", e)
                ))?;
            
            let _ = child.wait().await;
        }

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
        Ok(stage.clone())
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
        Some("connected".to_string())
    } else if line_lower.contains("connecting") || line_lower.contains("waiting") {
        Some("connecting".to_string())
    } else if line_lower.contains("disconnecting") || line_lower.contains("exiting") {
        Some("disconnecting".to_string())
    } else if line_lower.contains("authentication") || line_lower.contains("auth") {
        Some("authenticating".to_string())
    } else if line_lower.contains("error") || line_lower.contains("failed") {
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

