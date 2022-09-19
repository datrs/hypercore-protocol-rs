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

/// Encode data into a buffer.
///
/// This trait is implemented on data frames and their components
/// (channel messages, messages, and individual message types through prost).
pub trait Encoder: Sized + fmt::Debug {
    /// Calculates the length that the encoded message needs.
    fn encoded_len(&self) -> usize;

    /// Encodes the message to a buffer.
    ///
    /// An error will be returned if the buffer does not have sufficient capacity.
    fn encode(&self, buf: &mut [u8]) -> Result<usize, EncodeError>;
}

impl Encoder for &[u8] {
    fn encoded_len(&self) -> usize {
        self.len()
    }

    fn encode(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if len > buf.len() {
            return Err(EncodeError::new(len));
        }
        buf[..len].copy_from_slice(&self[..]);
        Ok(len)
    }
}

/// The type of a data frame.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameType {
    Raw,
    Message,
}
