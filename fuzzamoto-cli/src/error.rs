use std::fmt;

#[derive(Debug)]
pub enum CliError {
    IoError(std::io::Error),
    JsonError(serde_json::Error),
    YamlError(serde_yaml::Error),
    PostcardError(postcard::Error),
    ProcessError(String),
    InvalidInput(String),
    ShareDirExists,
    FileNotFound(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::IoError(e) => write!(f, "IO error: {}", e),
            CliError::JsonError(e) => write!(f, "JSON error: {}", e),
            CliError::YamlError(e) => write!(f, "YAML error: {}", e),
            CliError::PostcardError(e) => write!(f, "Postcard error: {}", e),
            CliError::ProcessError(msg) => write!(f, "Process error: {}", msg),
            CliError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            CliError::ShareDirExists => write!(f, "Share directory already exists"),
            CliError::FileNotFound(path) => write!(f, "File not found: {}", path),
        }
    }
}

impl std::error::Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        CliError::IoError(error)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(error: serde_json::Error) -> Self {
        CliError::JsonError(error)
    }
}

impl From<serde_yaml::Error> for CliError {
    fn from(error: serde_yaml::Error) -> Self {
        CliError::YamlError(error)
    }
}

impl From<postcard::Error> for CliError {
    fn from(error: postcard::Error) -> Self {
        CliError::PostcardError(error)
    }
}

pub type Result<T> = std::result::Result<T, CliError>;
