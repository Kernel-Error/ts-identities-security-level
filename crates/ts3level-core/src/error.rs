use std::path::PathBuf;
use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error on {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("io error: {0}")]
    PlainIo(#[from] std::io::Error),

    #[error("no [Identity] section or `identity=` key found")]
    NoIdentityKey,

    #[error("`identity` value is not quoted")]
    UnquotedIdentity,

    #[error("`identity` value contains no `V` separator")]
    NoCounterSeparator,

    #[error("counter is not a non-negative decimal integer: {0}")]
    InvalidCounter(String),

    #[error("identity blob is not valid base64: {0}")]
    BadBlobBase64(#[from] base64::DecodeError),

    #[error("deobfuscated payload is not valid base64")]
    BadInnerBase64,

    #[error("identity blob too short ({0} bytes) — needs at least 20 for the SHA-1 mask")]
    BlobTooShort(usize),

    #[error("ASN.1 DER: {0}")]
    Asn1(String),

    #[error("file lock contended: another process holds an exclusive lock on {0:?}")]
    Locked(PathBuf),

    #[error("file is not a valid UTF-8 ini: {0}")]
    NotUtf8(#[from] std::str::Utf8Error),
}
