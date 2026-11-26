//! Passphrase protection for export files.
//!
//! An export holds window titles, folder paths, tab addresses and notes, so it
//! can be sealed with a passphrase before it leaves this computer:
//! AES-256-GCM, with the key derived from the passphrase by Argon2id and a
//! random salt and nonce per file. A wrong passphrase or a changed file fails
//! to open rather than producing garbage.
//!
//! Layout: `CFXSEAL1` | salt (16) | nonce (12) | ciphertext.

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Result};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"CFXSEAL1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
/// Shortest passphrase accepted.
pub const MIN_PASSPHRASE: usize = 8;

/// A sealed file was opened without a passphrase.
#[derive(Debug)]
pub struct PassphraseRequired;

impl std::fmt::Display for PassphraseRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "this export is protected with a passphrase")
    }
}

impl std::error::Error for PassphraseRequired {}

/// Whether `bytes` are a sealed file.
pub fn is_sealed(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// The encryption key for `passphrase`. It is wiped from memory when dropped,
/// so it doesn't linger in the process after the export is written or read.
/// (The passphrase itself comes in as ordinary text from the interface, with
/// copies this program can't reach, so only the key is wiped here.)
fn key(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    let mut key = Zeroizing::new([0u8; 32]);
    argon2::Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut *key)
        .map_err(|e| anyhow!("couldn't derive a key from the passphrase: {e}"))?;
    Ok(key)
}

/// Seal `plain` with `passphrase`.
pub fn seal(plain: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    if passphrase.chars().count() < MIN_PASSPHRASE {
        bail!("use a passphrase of at least {MIN_PASSPHRASE} characters");
    }
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    let cipher = Aes256Gcm::new_from_slice(&*key(passphrase, &salt)?)?;
    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), plain)
        .map_err(|_| anyhow!("couldn't protect the file"))?;
    let mut out = Vec::with_capacity(MAGIC.len() + SALT_LEN + NONCE_LEN + sealed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Open a sealed file. Fails on a wrong passphrase or a changed file.
pub fn open(bytes: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let header = MAGIC.len() + SALT_LEN + NONCE_LEN;
    if !is_sealed(bytes) || bytes.len() < header {
        bail!("this isn't a passphrase-protected ContextFlow export");
    }
    let salt = &bytes[MAGIC.len()..MAGIC.len() + SALT_LEN];
    let nonce = &bytes[MAGIC.len() + SALT_LEN..header];
    let cipher = Aes256Gcm::new_from_slice(&*key(passphrase, salt)?)?;
    cipher
        .decrypt(Nonce::from_slice(nonce), &bytes[header..])
        .map_err(|_| anyhow!("wrong passphrase, or the file has been changed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_file_opens_only_with_its_passphrase() {
        let sealed = seal(b"{\"workspaces\":[]}", "correct horse").unwrap();
        assert!(is_sealed(&sealed));
        assert!(
            !sealed.windows(10).any(|w| w == b"workspaces"),
            "nothing readable inside"
        );
        assert_eq!(
            open(&sealed, "correct horse").unwrap(),
            b"{\"workspaces\":[]}"
        );
        assert!(open(&sealed, "wrong horse!").is_err());
        let mut changed = sealed.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert!(
            open(&changed, "correct horse").is_err(),
            "tampering is detected"
        );
    }

    #[test]
    fn short_passphrases_are_refused_and_each_file_is_different() {
        assert!(seal(b"x", "short").is_err());
        assert_ne!(
            seal(b"x", "long enough").unwrap(),
            seal(b"x", "long enough").unwrap()
        );
    }
}
