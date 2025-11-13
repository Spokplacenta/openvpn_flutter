//! Error handling for the OpenVPN module

use thiserror::Error;

#[derive(Error, Debug)]
pub enum OpenVpnError {
    #[error("Binary not found: {0}")]
    BinaryNotFound(String),
    
    #[error("Process execution failed: {0}")]
    ProcessExecutionFailed(String),
    
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
    
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("UTF-8 conversion error")]
    Utf8Error,
}

