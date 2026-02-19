/// Error type for this crate
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Error from the [`snow`] crate
    #[error("Error from `snow`: {0}")]
    Snow(#[from] snow::Error),
    /// Error from [`crypto_secretstream`] crate
    #[error("Error from `crypto_secretstream`: {0}")]
    SecretStream(crypto_secretstream::aead::Error),
    /// Error from [`std::io`]
    #[error("{0}")]
    FromStdIo(#[from] std::io::Error),
}

impl From<crypto_secretstream::aead::Error> for Error {
    fn from(value: crypto_secretstream::aead::Error) -> Self {
        Error::SecretStream(value)
    }
}

impl From<Error> for std::io::Error {
    fn from(value: Error) -> Self {
        std::io::Error::other(value)
    }
}
