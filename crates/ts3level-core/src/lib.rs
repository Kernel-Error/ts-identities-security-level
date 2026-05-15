//! Core primitives for working with TeamSpeak 3 identity files.
//!
//! - [`ini::IdentityFile`] parses and serializes the `.ini` byte-identically
//!   except for the counter digits.
//! - [`deobfuscate`] implements the TS3 XOR + SHA-1 obfuscation as documented
//!   in `landave/TSIdentityTool`.
//! - [`pubkey`] parses the libtomcrypt ASN.1 DER keypair and re-emits a
//!   public-only DER (the hash input the server expects).
//! - [`level`] is the reference CPU implementation of the security-level
//!   formula. The CUDA backend's output is verified against it in tests.
//! - [`writer`] performs the atomic, locked, backup-once file replace.

pub mod deobfuscate;
pub mod error;
pub mod ini;
pub mod level;
pub mod pubkey;
pub mod sha1_block;
pub mod writer;

pub use error::{Error, Result};
pub use ini::IdentityFile;
pub use level::{compute_level, level_of_hash};
pub use sha1_block::{sha1_block, SHA1_INIT};
