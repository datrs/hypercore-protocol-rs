use std::fmt;
use std::io;

/// Error if the buffer has insufficient size to encode a message.
#[derive(Debug)]
pub struct EncodeError {
    required: usize,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cannot encode message: Write buffer is full")
    }
}

impl EncodeError {
    pub fn new(required: usize) -> Self {
        Self { required }
    }
}

impl From<EncodeError> for io::Error {
    fn from(e: EncodeError) -> Self {
        io::Error::new(io::ErrorKind::Other, format!("{}", e))
    }
}

/// The type of a data frame.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameType {
    Raw,
    Message,
}
