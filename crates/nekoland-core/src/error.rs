use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum NekolandError {
    Io(std::io::Error),
    Config(String),
    Runtime(String),
}

impl Display for NekolandError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::Config(message) => write!(f, "config error: {message}"),
            Self::Runtime(message) => write!(f, "runtime error: {message}"),
        }
    }
}

impl Error for NekolandError {}

impl From<std::io::Error> for NekolandError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
