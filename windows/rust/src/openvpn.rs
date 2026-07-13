//! OpenVPN process management on Windows

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(target_os = "windows")]
use tokio::process::{Child as TokioChild, Command as TokioCommand};
use tokio::io::{AsyncBufReadExt, BufReader};
use crate::error::OpenVpnError;
use crate::service_ipc;

// ── Helpers ─────────────────────────────────────────────────────────────────

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

fn diag_log_to_file(msg: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::temp_dir().join("openvpn_rust_diag.log"))
    {
        let _ = writeln!(file, "{}", msg);
        let _ = file.flush();
    }
}

fn sanitize_for_log(line: &str) -> String {
    let collapsed = line.replace('\r', " ").replace('\n', " ");
    let trimmed = collapsed.trim();
    const MAX_LEN: usize = 240;
    if trimmed.len() > MAX_LEN {
        format!("{}...", &trimmed[..MAX_LEN])
    } else {
        trimmed.to_string()
    }
}

fn classify_reason(line: &str) -> &'static str {
    let line_lower = line.to_lowercase();
    if line_lower.contains("auth_failed") || line_lower.contains("authentication failed") {
        "AUTH_FAILED"
    } else if line_lower.contains("wintun requires system privileges")
        || line_lower.contains("should be used with interactive service")
    {
        "WINTUN_SYSTEM_PRIVILEGE_REQUIRED"
    } else if line_lower.contains("tls handshake failed") || line_lower.contains("tls error") {
        "TLS_FAILED"
    } else if line_lower.contains("cannot resolve host address")
        || line_lower.contains("resolve error")
    {
        "DNS_RESOLVE_FAILED"
    } else if line_lower.contains("connection timed out") {
        "CONNECT_TIMEOUT"
    } else if line_lower.contains("connection refused") {
        "CONNECTION_REFUSED"
    } else if line_lower.contains("certificate verify failed")
        || line_lower.contains("verify error")
    {
        "CERT_VERIFY_FAILED"
    } else if line_lower.contains("wintun")
        && line_lower.contains("cannot create wintun adapter")
    {
        "WINTUN_CREATE_FAILED"
    } else if line_lower.contains("network is unreachable") {
        "NETWORK_UNREACHABLE"
    } else if line_lower.contains("fatal") {
        "FATAL"
    } else if line_lower.contains("service ipc") || line_lower.contains("pipe") {
        "SERVICE_IPC_FAILED"
    } else {
        "UNKNOWN"
    }
}

fn log_stage_transition(stream: &str, from: &str, to: &str, line: &str, start: Instant) {
    let elapsed_ms = start.elapsed().as_millis();
    let reason_code = classify_reason(line);
    let trigger_line = sanitize_for_log(line);
    diag_log_to_file(&format!(
        "[RUST_STAGE] stream={} from={} to={} elapsedMs={} reasonCode={} trigger=\"{}\"",
        stream, from, to, elapsed_ms, reason_code, trigger_line
    ));
}

async fn kill_all_openvpn_processes() {
    debug_log_to_file("[DEBUG] Attempting to kill all OpenVPN processes...");
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
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("not found") || stderr.contains("not running") {
                    debug_log_to_file("[DEBUG] No OpenVPN processes found to kill");
                } else {
                    debug_log_to_file(&format!("[DEBUG] Warning: taskkill failed: {}", stderr));
                }
            }
        }
        Err(e) => {
            debug_log_to_file(&format!(
                "[DEBUG] Warning: Failed to execute taskkill: {}",
                e
            ));
        }
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
}

async fn restart_interactive_service() -> bool {
    diag_log_to_file("[RUST_SERVICE] restarting OpenVPNServiceInteractive...");

    let stop = {
        let mut cmd = TokioCommand::new("net");
        cmd.args(["stop", "OpenVPNServiceInteractive"]);
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.output().await
    };
    if let Ok(ref out) = stop {
        let msg = String::from_utf8_lossy(&out.stdout);
        diag_log_to_file(&format!("[RUST_SERVICE] stop: {}", msg.trim()));
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    let start = {
        let mut cmd = TokioCommand::new("net");
        cmd.args(["start", "OpenVPNServiceInteractive"]);
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.output().await
    };
    let ok = match start {
        Ok(ref out) => {
            let msg = String::from_utf8_lossy(&out.stdout);
            diag_log_to_file(&format!("[RUST_SERVICE] start: {}", msg.trim()));
            out.status.success()
        }
        Err(ref e) => {
            diag_log_to_file(&format!("[RUST_SERVICE] start failed: {}", e));
            false
        }
    };

    tokio::time::sleep(Duration::from_millis(200)).await;
    ok
}

async fn kill_process_by_pid(pid: u32) {
    debug_log_to_file(&format!("[DEBUG] Killing PID {}...", pid));
    let mut cmd = TokioCommand::new("taskkill");
    cmd.args(["/F", "/PID", &pid.to_string()]);
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    match cmd.output().await {
        Ok(out) => {
            let msg = String::from_utf8_lossy(&out.stdout);
            debug_log_to_file(&format!("[DEBUG] taskkill PID {}: {}", pid, msg.trim()));
        }
        Err(e) => {
            debug_log_to_file(&format!("[DEBUG] taskkill PID {} error: {}", pid, e));
        }
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
}

// ── Types ───────────────────────────────────────────────────────────────────

/// VPN connection statistics
#[derive(Clone, Default)]
pub struct VpnStats {
    pub byte_in: u64,
    pub byte_out: u64,
    pub packets_in: u64,
    pub packets_out: u64,
    pub connected_on: Option<std::time::SystemTime>,
}

/// Tracks how the current openvpn.exe was launched.
enum ManagedProcess {
    /// Launched directly via `tokio::process::Command` (stdout/stderr piped).
    Direct(TokioChild),
    /// Launched via the Interactive Service IPC; only the PID is known.
    Service { pid: u32 },
}

// ── Manager ─────────────────────────────────────────────────────────────────

pub struct OpenVpnManager {
    binary_path: PathBuf,
    process: Arc<Mutex<Option<ManagedProcess>>>,
    current_stage: Arc<Mutex<String>>,
    config_file: Arc<Mutex<Option<PathBuf>>>,
    auth_file: Arc<Mutex<Option<PathBuf>>>,
    status_file: Arc<Mutex<Option<PathBuf>>>,
    log_file: Arc<Mutex<Option<PathBuf>>>,
    stats: Arc<Mutex<VpnStats>>,
}

impl OpenVpnManager {
    pub fn new(binary_path: String) -> Result<Self, OpenVpnError> {
        debug_log_to_file(&format!(
            "[DEBUG] OpenVpnManager::new() binary_path: {}",
            binary_path
        ));
        let path = PathBuf::from(&binary_path);
        if !path.exists() {
            return Err(OpenVpnError::BinaryNotFound(format!(
                "OpenVPN binary not found at: {}",
                path.display()
            )));
        }
        debug_log_to_file("[DEBUG] OpenVpnManager created successfully");
        Ok(OpenVpnManager {
            binary_path: path,
            process: Arc::new(Mutex::new(None)),
            current_stage: Arc::new(Mutex::new("disconnected".to_string())),
            config_file: Arc::new(Mutex::new(None)),
            auth_file: Arc::new(Mutex::new(None)),
            status_file: Arc::new(Mutex::new(None)),
            log_file: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(VpnStats::default())),
        })
    }

    /// Runtime directories for configs/logs shared with the Interactive Service.
    /// Must live under ProgramData so LocalSystem can access them.
    fn runtime_dirs(&self) -> (PathBuf, PathBuf, PathBuf) {
        #[cfg(target_os = "windows")]
        {
            let base = std::env::var("ProgramData")
                .map(|p| PathBuf::from(p).join("LavControl").join("OpenVPN"))
                .unwrap_or_else(|_| PathBuf::from(r"C:\ProgramData\LavControl\OpenVPN"));
            return (
                base.join("config"),
                base.join("log"),
                base.join("status"),
            );
        }
        #[cfg(not(target_os = "windows"))]
        {
            let base = self
                .binary_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            (
                base.join("config"),
                base.join("log"),
                base.join("status"),
            )
        }
    }

    fn ensure_runtime_dirs(
        config_dir: &PathBuf,
        log_dir: &PathBuf,
        status_dir: &PathBuf,
    ) -> Result<(), OpenVpnError> {
        for dir in [config_dir, log_dir, status_dir] {
            std::fs::create_dir_all(dir).map_err(OpenVpnError::IoError)?;
        }
        Ok(())
    }

    // ── connect ─────────────────────────────────────────────────────────

    pub async fn connect(
        &self,
        config: &str,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<(), OpenVpnError> {
        let connect_started_at = Instant::now();
        debug_log_to_file("[DEBUG] OpenVpnManager::connect() called");
        diag_log_to_file(&format!(
            "[RUST_CONNECT] phase=start configBytes={} hasUsername={} hasPassword={}",
            config.len(),
            username.is_some(),
            password.is_some()
        ));

        // ── 1. Clean up any previous connection ─────────────────────────
        kill_all_openvpn_processes().await;
        {
            let mut process = self.process.lock().unwrap();
            if let Some(managed) = process.take() {
                match managed {
                    ManagedProcess::Direct(mut child) => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                    }
                    ManagedProcess::Service { pid, .. } => {
                        kill_process_by_pid(pid).await;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;

        // ── 2. Write runtime files under the OpenVPN runtime directory ──
        // The Interactive Service rejects configs outside `config_dir`.
        let (config_dir, log_dir, status_dir) = self.runtime_dirs();
        Self::ensure_runtime_dirs(&config_dir, &log_dir, &status_dir)?;

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let config_file = config_dir.join(format!("openvpn_config_{}.ovpn", ts));
        std::fs::write(&config_file, config).map_err(OpenVpnError::IoError)?;
        debug_log_to_file(&format!(
            "[DEBUG] Config written: {}",
            config_file.display()
        ));

        let auth_file = if username.is_some() && password.is_some() {
            let content = format!(
                "{}\n{}",
                username.as_ref().unwrap(),
                password.as_ref().unwrap()
            );
            let path = config_dir.join(format!("openvpn_auth_{}.txt", ts));
            std::fs::write(&path, content).map_err(OpenVpnError::IoError)?;
            Some(path)
        } else {
            None
        };

        let status_file_path = status_dir.join(format!("openvpn_status_{}.txt", ts));

        // Store common temp paths
        {
            *self.config_file.lock().unwrap() = Some(config_file.clone());
        }
        if let Some(ref ap) = auth_file {
            *self.auth_file.lock().unwrap() = Some(ap.clone());
        }
        {
            *self.status_file.lock().unwrap() = Some(status_file_path.clone());
        }
        {
            let mut stats = self.stats.lock().unwrap();
            stats.connected_on = Some(std::time::SystemTime::now());
        }

        // ── 3. Try IPC Interactive Service path ─────────────────────────
        // Only restart the service when the pipe is unavailable (avoids
        // pointless net stop/start attempts from a non-elevated process).
        let service_available_before = service_ipc::is_service_available();
        let service_restarted = if !service_available_before {
            restart_interactive_service().await
        } else {
            false
        };
        let service_available = service_ipc::is_service_available();
        diag_log_to_file(&format!(
            "[RUST_CONNECT] phase=service_check serviceAvailableBefore={} serviceRestarted={} serviceAvailable={}",
            service_available_before, service_restarted, service_available
        ));

        let mut used_service = false;

        if service_available {
            let log_file_path = log_dir.join(format!("openvpn_log_{}.txt", ts));
            {
                *self.log_file.lock().unwrap() = Some(log_file_path.clone());
            }

            // Build CLI options — always force wintun when going through the
            // service (it runs as SYSTEM so has the required privileges).
            let mut opts = format!("--config \"{}\"", config_file.display());
            if let Some(ref ap) = auth_file {
                opts.push_str(&format!(" --auth-user-pass \"{}\"", ap.display()));
            }
            opts.push_str(" --verb 4");
            opts.push_str(&format!(" --status \"{}\" 2", status_file_path.display()));
            opts.push_str(&format!(" --log \"{}\"", log_file_path.display()));

            let config_lower = config.to_lowercase();
            if !config_lower.contains("windows-driver") {
                opts.push_str(" --windows-driver wintun");
            }

            let working_dir = config_dir.to_string_lossy().to_string();

            diag_log_to_file(&format!(
                "[RUST_CONNECT] phase=service_ipc_attempt workingDir=\"{}\" optsLen={}",
                working_dir,
                opts.len()
            ));

            match service_ipc::connect_and_launch(&working_dir, &opts, "") {
                Ok(response) => {
                    diag_log_to_file(&format!(
                        "[RUST_CONNECT] phase=service_ipc_ok pid={} desc=\"{}\" driverStrategy=wintun_via_service elapsedMs={}",
                        response.pid,
                        response.description,
                        connect_started_at.elapsed().as_millis()
                    ));

                    {
                        let mut process = self.process.lock().unwrap();
                        *process = Some(ManagedProcess::Service {
                            pid: response.pid,
                        });
                    }

                    // Spawn log file tailer task (replaces stdout/stderr readers)
                    self.spawn_log_tailer(
                        log_file_path,
                        response.pid,
                        connect_started_at,
                    );

                    used_service = true;
                }
                Err(e) => {
                    diag_log_to_file(&format!(
                        "[RUST_CONNECT] phase=service_ipc_failed error=\"{}\" fallback=direct",
                        e
                    ));
                    // Clear log_file — we won't use it in direct mode.
                    *self.log_file.lock().unwrap() = None;
                }
            }
        }

        // ── 4. Fallback: direct launch ──────────────────────────────────
        if !used_service {
            self.connect_direct(
                config,
                &config_file,
                &auth_file,
                &status_file_path,
                connect_started_at,
            )
            .await?;
        }

        // ── 5. Spawn status file reader (common) ────────────────────────
        self.spawn_status_reader();

        diag_log_to_file(&format!(
            "[RUST_CONNECT] phase=return_ok mode={} elapsedMs={}",
            if used_service { "service" } else { "direct" },
            connect_started_at.elapsed().as_millis()
        ));

        Ok(())
    }

    // ── Direct launch (existing behaviour) ──────────────────────────────

    async fn connect_direct(
        &self,
        config: &str,
        config_file: &PathBuf,
        auth_file: &Option<PathBuf>,
        status_file_path: &PathBuf,
        connect_started_at: Instant,
    ) -> Result<(), OpenVpnError> {
        let mut cmd = TokioCommand::new(&self.binary_path);
        cmd.arg("--config").arg(config_file);

        if let Some(ref auth_path) = auth_file {
            cmd.arg("--auth-user-pass").arg(auth_path);
        }

        cmd.arg("--verb").arg("4");

        // In direct mode, Wintun can never work (it requires SYSTEM privileges,
        // not just admin elevation). Always use tap-windows6 unless the config
        // explicitly specifies a driver.
        let config_lower = config.to_lowercase();
        if !config_lower.contains("windows-driver") {
            cmd.arg("--windows-driver").arg("tap-windows6");
            diag_log_to_file(
                "[RUST_CONNECT] driverStrategy=tap-windows6_direct_mode",
            );
        } else {
            diag_log_to_file(
                "[RUST_CONNECT] driverStrategy=config_defined mode=direct",
            );
        }

        cmd.arg("--status").arg(status_file_path).arg("2");

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| {
            OpenVpnError::ProcessExecutionFailed(format!(
                "Failed to spawn OpenVPN process: {}",
                e
            ))
        })?;

        let pid = child.id();
        diag_log_to_file(&format!(
            "[RUST_CONNECT] phase=spawn_ok pid={:?} mode=direct elapsedMs={}",
            pid,
            connect_started_at.elapsed().as_millis()
        ));

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // stdout reader
        let stage_stdout = Arc::clone(&self.current_stage);
        let stats_stdout = Arc::clone(&self.stats);
        let start_stdout = connect_started_at;
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if let Some(new_stage) = parse_stage_from_output(&line) {
                            let mut current = stage_stdout.lock().unwrap();
                            let old = current.clone();
                            if old != new_stage {
                                log_stage_transition(
                                    "stdout",
                                    &old,
                                    &new_stage,
                                    &line,
                                    start_stdout,
                                );
                                *current = new_stage.clone();
                                if new_stage == "connected" {
                                    let mut stats = stats_stdout.lock().unwrap();
                                    if stats.connected_on.is_none() {
                                        stats.connected_on =
                                            Some(std::time::SystemTime::now());
                                    }
                                }
                            }
                        }
                        parse_stats_from_output(&line, &stats_stdout);
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        });

        // stderr reader
        let stage_stderr = Arc::clone(&self.current_stage);
        let stats_stderr = Arc::clone(&self.stats);
        let start_stderr = connect_started_at;
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        debug_log_to_file(&format!("[DEBUG] stderr: {}", line));
                        if let Some(new_stage) = parse_stage_from_output(&line) {
                            let mut current = stage_stderr.lock().unwrap();
                            let old = current.clone();
                            if old != new_stage {
                                log_stage_transition(
                                    "stderr",
                                    &old,
                                    &new_stage,
                                    &line,
                                    start_stderr,
                                );
                                *current = new_stage.clone();
                                if new_stage == "connected" {
                                    let mut stats = stats_stderr.lock().unwrap();
                                    if stats.connected_on.is_none() {
                                        stats.connected_on =
                                            Some(std::time::SystemTime::now());
                                    }
                                }
                            }
                        }
                        parse_stats_from_output(&line, &stats_stderr);
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        });

        // Store the child handle
        {
            let mut process = self.process.lock().unwrap();
            *process = Some(ManagedProcess::Direct(child));
        }

        Ok(())
    }

    // ── Log file tailer task (Service mode) ─────────────────────────────

    fn spawn_log_tailer(
        &self,
        log_path: PathBuf,
        service_pid: u32,
        started_at: Instant,
    ) {
        let log_file_arc = Arc::clone(&self.log_file);
        let stage_arc = Arc::clone(&self.current_stage);
        let stats_arc = Arc::clone(&self.stats);

        tokio::spawn(async move {
            let mut tailer = service_ipc::LogFileTailer::new(log_path);
            loop {
                tokio::time::sleep(Duration::from_millis(300)).await;

                // Stop signal: log_file set to None by disconnect().
                {
                    let guard = log_file_arc.lock().unwrap();
                    if guard.is_none() {
                        break;
                    }
                }

                for line in tailer.read_new_lines() {
                    if let Some(new_stage) = parse_stage_from_output(&line) {
                        let mut current = stage_arc.lock().unwrap();
                        let old = current.clone();
                        if old != new_stage {
                            log_stage_transition(
                                "log_tailer",
                                &old,
                                &new_stage,
                                &line,
                                started_at,
                            );
                            *current = new_stage.clone();
                            if new_stage == "connected" {
                                let mut stats = stats_arc.lock().unwrap();
                                if stats.connected_on.is_none() {
                                    stats.connected_on =
                                        Some(std::time::SystemTime::now());
                                }
                            }
                        }
                    }
                }

                // Detect process death.
                if !service_ipc::is_pid_alive(service_pid) {
                    let mut current = stage_arc.lock().unwrap();
                    if *current != "connected"
                        && *current != "disconnected"
                        && *current != "disconnecting"
                    {
                        diag_log_to_file(&format!(
                            "[RUST_STAGE] stream=log_tailer from={} to=error elapsedMs={} reasonCode=PROCESS_DIED trigger=\"PID {} gone\"",
                            *current,
                            started_at.elapsed().as_millis(),
                            service_pid
                        ));
                        *current = "error".to_string();
                    }
                    break;
                }
            }
            diag_log_to_file("[RUST_LOG_TAILER] task exited");
        });
    }

    // ── Status file reader task (common) ────────────────────────────────

    fn spawn_status_reader(&self) {
        let stats_arc = Arc::clone(&self.stats);
        let status_file_arc = Arc::clone(&self.status_file);

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let path = {
                    let guard = status_file_arc.lock().unwrap();
                    guard.clone()
                };
                match path {
                    Some(ref p) => {
                        if let Ok(content) = std::fs::read_to_string(p) {
                            parse_status_file(&content, &stats_arc);
                        }
                    }
                    None => break,
                }
            }
        });
    }

    // ── disconnect ──────────────────────────────────────────────────────

    pub async fn disconnect(&self) -> Result<(), OpenVpnError> {
        debug_log_to_file("[DEBUG] OpenVpnManager::disconnect() called");

        let managed = {
            let mut process = self.process.lock().unwrap();
            process.take()
        };

        match managed {
            Some(ManagedProcess::Direct(mut child)) => {
                kill_all_openvpn_processes().await;
                let _ = child.kill().await;
                let _ = child.wait().await;
                debug_log_to_file("[DEBUG] Direct process cleaned up");
            }
            Some(ManagedProcess::Service { pid, .. }) => {
                diag_log_to_file(&format!(
                    "[RUST_DISCONNECT] mode=service pid={}",
                    pid
                ));
                kill_process_by_pid(pid).await;
            }
            None => {
                debug_log_to_file("[DEBUG] No tracked process to disconnect");
            }
        }

        tokio::time::sleep(Duration::from_millis(1500)).await;

        // Clean up temp files
        Self::cleanup_file(&self.config_file);
        Self::cleanup_file(&self.auth_file);
        Self::cleanup_file(&self.status_file);
        Self::cleanup_file(&self.log_file);

        // Reset stage / stats
        *self.current_stage.lock().unwrap() = "disconnected".to_string();
        *self.stats.lock().unwrap() = VpnStats::default();

        Ok(())
    }

    fn cleanup_file(holder: &Arc<Mutex<Option<PathBuf>>>) {
        let mut guard = holder.lock().unwrap();
        if let Some(ref path) = *guard {
            let _ = std::fs::remove_file(path);
        }
        *guard = None;
    }

    // ── Getters ─────────────────────────────────────────────────────────

    pub fn get_stage(&self) -> Result<String, OpenVpnError> {
        let stage = self.current_stage.lock().unwrap().clone();
        debug_log_to_file(&format!("[DEBUG] get_stage() -> {}", stage));
        Ok(stage)
    }

    pub fn get_stats(&self) -> Result<VpnStats, OpenVpnError> {
        Ok(self.stats.lock().unwrap().clone())
    }
}

// ── Output parsing ──────────────────────────────────────────────────────────

fn parse_stage_from_output(line: &str) -> Option<String> {
    let line_lower = line.to_lowercase();

    if line_lower.contains("initialization sequence completed") {
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
        || line_lower.contains("verify error")
    {
        Some("error".to_string())
    } else if line_lower.contains("connecting") || line_lower.contains("waiting") {
        Some("connecting".to_string())
    } else if line_lower.contains("disconnecting") || line_lower.contains("exiting") {
        Some("disconnecting".to_string())
    } else if line_lower.contains("authentication") || line_lower.contains("auth") {
        Some("authenticating".to_string())
    } else if line_lower.contains("preserving previous tun/tap instance") {
        None
    } else if (line_lower.contains("wintun") || line_lower.contains("tun/tap"))
        && (line_lower.contains("cannot create wintun adapter")
            || line_lower.contains("wintun.dll")
            || line_lower.contains("cannot allocate tun/tap")
            || line_lower.contains("error_gen_failure"))
    {
        Some("error".to_string())
    } else if line_lower.contains("error") || line_lower.contains("failed") {
        Some("error".to_string())
    } else {
        None
    }
}

fn parse_stats_from_output(line: &str, stats: &Arc<Mutex<VpnStats>>) {
    let line_lower = line.to_lowercase();

    if line_lower.contains("read bytes") || line_lower.contains("bytes read") {
        if let Some(bytes) = extract_number_after_keyword(&line_lower, "read bytes") {
            stats.lock().unwrap().byte_in = bytes;
        } else if let Some(bytes) = extract_number_after_keyword(&line_lower, "bytes read") {
            stats.lock().unwrap().byte_in = bytes;
        }
    }
    if line_lower.contains("write bytes") || line_lower.contains("bytes write") {
        if let Some(bytes) = extract_number_after_keyword(&line_lower, "write bytes") {
            stats.lock().unwrap().byte_out = bytes;
        } else if let Some(bytes) = extract_number_after_keyword(&line_lower, "bytes write") {
            stats.lock().unwrap().byte_out = bytes;
        }
    }
}

fn parse_status_file(content: &str, stats: &Arc<Mutex<VpnStats>>) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("OpenVPN STATISTICS")
            || line.starts_with("Updated,")
        {
            continue;
        }
        if let Some(val) = line.strip_prefix("TUN/TAP read bytes,") {
            if let Ok(b) = val.trim().parse::<u64>() {
                stats.lock().unwrap().byte_in = b;
            }
        } else if let Some(val) = line.strip_prefix("TUN/TAP write bytes,") {
            if let Ok(b) = val.trim().parse::<u64>() {
                stats.lock().unwrap().byte_out = b;
            }
        } else if let Some(val) = line.strip_prefix("TCP/UDP read bytes,") {
            if let Ok(b) = val.trim().parse::<u64>() {
                stats.lock().unwrap().byte_in = b;
            }
        } else if let Some(val) = line.strip_prefix("TCP/UDP write bytes,") {
            if let Ok(b) = val.trim().parse::<u64>() {
                stats.lock().unwrap().byte_out = b;
            }
        }
    }
}

fn extract_number_after_keyword(line: &str, keyword: &str) -> Option<u64> {
    let pos = line.find(keyword)?;
    let after = &line[pos + keyword.len()..];
    let number_str: String = after
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(|c| c.is_ascii_digit())
        .collect();
    if number_str.is_empty() {
        None
    } else {
        number_str.parse::<u64>().ok()
    }
}
