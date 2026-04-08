//! OpenVPN Management Interface TCP client
//!
//! Connects to the OpenVPN management socket to receive real-time
//! state/bytecount notifications and send commands (e.g. SIGTERM).

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::error::OpenVpnError;
use crate::openvpn::VpnStats;

#[derive(Debug, Clone)]
pub enum ManagementEvent {
    State {
        name: String,
        description: String,
        local_ip: Option<String>,
    },
    ByteCount {
        bytes_in: u64,
        bytes_out: u64,
    },
    Log {
        message: String,
    },
    ProcessExited,
}

pub struct ManagementClient {
    writer: Arc<tokio::sync::Mutex<tokio::io::WriteHalf<TcpStream>>>,
    port: u16,
}

impl ManagementClient {
    /// Connect to the management interface, authenticate with `password`,
    /// and enable real-time notifications.
    pub async fn connect(
        port: u16,
        password: &str,
        stage: Arc<Mutex<String>>,
        stats: Arc<Mutex<VpnStats>>,
    ) -> Result<Self, OpenVpnError> {
        let stream = Self::connect_with_retry(port, 30, Duration::from_millis(500)).await?;
        let (reader, writer) = tokio::io::split(stream);
        let writer = Arc::new(tokio::sync::Mutex::new(writer));

        let pwd = password.to_string();
        let writer_auth = Arc::clone(&writer);

        // Spawn reader task that handles auth then processes events.
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            let mut authenticated = false;

            while let Ok(Some(line)) = lines.next_line().await {
                if !authenticated {
                    // The management interface prints a greeting then asks
                    // "ENTER PASSWORD:" when `--management ... stdin` is used.
                    if line.contains("PASSWORD") || line.contains("password") {
                        let cmd = format!("{}\n", pwd);
                        let mut w = writer_auth.lock().await;
                        let _ = w.write_all(cmd.as_bytes()).await;
                        let _ = w.flush().await;
                        continue;
                    }
                    if line.contains("SUCCESS") || line.contains(">INFO") {
                        authenticated = true;
                        // Enable real-time notifications
                        let mut w = writer_auth.lock().await;
                        let _ = w.write_all(b"state on\n").await;
                        let _ = w.write_all(b"bytecount 2\n").await;
                        let _ = w.flush().await;
                        continue;
                    }
                    continue;
                }

                // Parse real-time notifications
                if let Some(event) = parse_management_line(&line) {
                    match event {
                        ManagementEvent::State {
                            ref name,
                            ..
                        } => {
                            let mapped = map_mgmt_state(name);
                            let mut s = stage.lock().unwrap();
                            *s = mapped;
                        }
                        ManagementEvent::ByteCount {
                            bytes_in,
                            bytes_out,
                        } => {
                            let mut st = stats.lock().unwrap();
                            st.byte_in = bytes_in;
                            st.byte_out = bytes_out;
                        }
                        ManagementEvent::ProcessExited => {
                            let mut s = stage.lock().unwrap();
                            *s = "disconnected".to_string();
                            break;
                        }
                        _ => {}
                    }
                }
            }

            // EOF — process has exited
            let mut s = stage.lock().unwrap();
            if *s != "disconnected" {
                *s = "disconnected".to_string();
            }
        });

        Ok(ManagementClient { writer, port })
    }

    /// Send `signal SIGTERM` to gracefully disconnect.
    pub async fn signal_terminate(&self) -> Result<(), OpenVpnError> {
        let mut w = self.writer.lock().await;
        w.write_all(b"signal SIGTERM\n")
            .await
            .map_err(OpenVpnError::IoError)?;
        w.flush().await.map_err(OpenVpnError::IoError)?;
        Ok(())
    }

    /// Send an arbitrary management command.
    #[allow(dead_code)]
    pub async fn send_command(&self, cmd: &str) -> Result<(), OpenVpnError> {
        let mut w = self.writer.lock().await;
        w.write_all(format!("{}\n", cmd).as_bytes())
            .await
            .map_err(OpenVpnError::IoError)?;
        w.flush().await.map_err(OpenVpnError::IoError)?;
        Ok(())
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    async fn connect_with_retry(
        port: u16,
        max_attempts: u32,
        delay: Duration,
    ) -> Result<TcpStream, OpenVpnError> {
        let addr = format!("127.0.0.1:{}", port);
        for attempt in 1..=max_attempts {
            match TcpStream::connect(&addr).await {
                Ok(stream) => return Ok(stream),
                Err(_) if attempt < max_attempts => {
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    return Err(OpenVpnError::ConnectionFailed(format!(
                        "Management interface connection failed after {} attempts on port {}: {}",
                        max_attempts, port, e
                    )));
                }
            }
        }
        unreachable!()
    }
}

/// Parse a single line from the management interface.
fn parse_management_line(line: &str) -> Option<ManagementEvent> {
    // >STATE:1620000000,CONNECTED,SUCCESS,10.8.0.2,1.2.3.4,1194,,
    if let Some(rest) = line.strip_prefix(">STATE:") {
        let parts: Vec<&str> = rest.splitn(8, ',').collect();
        if parts.len() >= 2 {
            let name = parts[1].to_string();
            let description = parts.get(2).unwrap_or(&"").to_string();
            let local_ip = parts.get(3).map(|s| s.to_string()).filter(|s| !s.is_empty());
            return Some(ManagementEvent::State {
                name,
                description,
                local_ip,
            });
        }
    }

    // >BYTECOUNT:12345,67890
    if let Some(rest) = line.strip_prefix(">BYTECOUNT:") {
        let parts: Vec<&str> = rest.splitn(2, ',').collect();
        if parts.len() == 2 {
            if let (Ok(bi), Ok(bo)) = (parts[0].parse::<u64>(), parts[1].parse::<u64>()) {
                return Some(ManagementEvent::ByteCount {
                    bytes_in: bi,
                    bytes_out: bo,
                });
            }
        }
    }

    // >LOG:timestamp,flags,message
    if let Some(rest) = line.strip_prefix(">LOG:") {
        let parts: Vec<&str> = rest.splitn(3, ',').collect();
        let message = parts.last().unwrap_or(&"").to_string();
        return Some(ManagementEvent::Log { message });
    }

    if line.contains("END") && line.contains("exit") {
        return Some(ManagementEvent::ProcessExited);
    }

    None
}

/// Map OpenVPN management state names to our internal stage strings.
fn map_mgmt_state(state: &str) -> String {
    match state.to_uppercase().as_str() {
        "CONNECTED" => "connected".to_string(),
        "CONNECTING" => "connecting".to_string(),
        "WAIT" => "wait_connection".to_string(),
        "AUTH" => "authenticating".to_string(),
        "GET_CONFIG" => "get_config".to_string(),
        "ASSIGN_IP" => "assign_ip".to_string(),
        "ADD_ROUTES" => "connecting".to_string(),
        "RECONNECTING" => "connecting".to_string(),
        "EXITING" => "disconnecting".to_string(),
        "RESOLVE" => "resolve".to_string(),
        "TCP_CONNECT" => "tcp_connect".to_string(),
        "UDP_CONNECT" => "udp_connect".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Find a free TCP port by binding to port 0.
pub fn find_free_port() -> Result<u16, OpenVpnError> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| OpenVpnError::ConfigurationError(format!("Cannot find free port: {}", e)))?;
    let port = listener.local_addr()
        .map_err(|e| OpenVpnError::ConfigurationError(format!("Cannot get local addr: {}", e)))?
        .port();
    Ok(port)
}

/// Generate a random alphanumeric password for the management interface.
pub fn generate_mgmt_password() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..24)
        .map(|_| {
            let idx = rng.gen_range(0..36);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect()
}
