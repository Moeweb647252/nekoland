use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

use crate::schema::NekolandConfigFile;

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(String),
    UnsupportedFormat(String),
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::Parse(message) => write!(f, "parse error: {message}"),
            Self::UnsupportedFormat(ext) => write!(f, "unsupported config format: {ext}"),
        }
    }
}

impl Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn load_from_path(path: impl AsRef<Path>) -> Result<NekolandConfigFile, ConfigError> {
    let path = path.as_ref();
    let contents = std::fs::read_to_string(path)?;
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();

    match extension {
        "toml" => toml::from_str(&contents).map_err(|error| ConfigError::Parse(error.to_string())),
        "ron" => ron::from_str(&contents).map_err(|error| ConfigError::Parse(error.to_string())),
        other => Err(ConfigError::UnsupportedFormat(other.to_owned())),
    }
}
