use std::error::Error as StdError;
use std::fmt;
use std::io::Error;
#[derive(Debug)]
pub enum ProgError {
    NoFile,
    NotUtf8,
    Io(Error),
}

// Implement Display
impl fmt::Display for ProgError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ProgError::NoFile => write!(f, "no file found"),
            ProgError::NotUtf8 => write!(f, "content is not valid UTF-8"),
            ProgError::Io(err) => write!(f, "IO error: {}", err),
        }
    }
}

// Implement Error
impl StdError for ProgError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            ProgError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<Error> for ProgError {
    fn from(err: Error) -> ProgError {
        ProgError::Io(err)
    }
}
