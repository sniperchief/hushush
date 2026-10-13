//! Seed handling. The BIP 39 mnemonic is the wallet's sole spend authority and
//! sole recovery secret. At rest it is encrypted with `age` (X25519) to an
//! identity file kept in a separate secrets directory. It is never logged and
//! never printed by any command except the explicit `backup-mnemonic`, which
//! writes it to a file the operator names.

use std::io::{Read, Write};
use std::path::Path;

use age::secrecy::ExposeSecret as _;
use anyhow::{Context, anyhow};
use bip0039::{Count, English, Mnemonic};
use secrecy::{ExposeSecret, SecretString, SecretVec, Zeroize};

pub fn generate_mnemonic() -> Mnemonic<English> {
    Mnemonic::generate(Count::Words24)
}

pub fn parse_mnemonic(phrase: &SecretString) -> anyhow::Result<Mnemonic<English>> {
    let normalised = phrase
        .expose_secret()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    <Mnemonic<English>>::from_phrase(normalised).map_err(|e| anyhow!("invalid mnemonic: {e}"))
}

/// BIP 39 seed (empty passphrase, matching Zashi/zcash-devtool conventions).
pub fn seed_from_mnemonic(mnemonic: &Mnemonic<English>) -> SecretVec<u8> {
    let mut seed = mnemonic.to_seed("");
    let secret = SecretVec::new(seed.to_vec());
    seed.zeroize();
    secret
}

/// Creates a new age identity at `path` (failing if one exists) and returns it.
pub fn create_identity(path: &Path) -> anyhow::Result<age::x25519::Identity> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let identity = age::x25519::Identity::generate();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating identity file {}", path.display()))?;
    writeln!(f, "# hushush phase0 wallet identity — DO NOT COMMIT, DO NOT SHARE")?;
    writeln!(f, "# public key: {}", identity.to_public())?;
    writeln!(f, "{}", identity.to_string().expose_secret())?;
    f.sync_all()?;
    restrict_permissions(path);
    Ok(identity)
}

pub fn load_identities(path: &Path) -> anyhow::Result<Vec<Box<dyn age::Identity>>> {
    let ids = age::IdentityFile::from_file(path.to_string_lossy().into_owned())
        .with_context(|| format!("reading identity file {}", path.display()))?
        .into_identities()?;
    Ok(ids
        .into_iter()
        .map(|i| i as Box<dyn age::Identity>)
        .collect())
}

pub fn encrypt_mnemonic(
    mnemonic: &Mnemonic<English>,
    recipient: &age::x25519::Recipient,
) -> anyhow::Result<Vec<u8>> {
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient))?;
    let mut out = vec![];
    let mut w = encryptor.wrap_output(&mut out)?;
    w.write_all(mnemonic.phrase().as_bytes())?;
    w.finish()?;
    Ok(out)
}

pub fn decrypt_mnemonic(
    ciphertext: &[u8],
    identities: &[Box<dyn age::Identity>],
) -> anyhow::Result<Mnemonic<English>> {
    let decryptor = age::Decryptor::new(ciphertext)?;
    let mut r = decryptor.decrypt(identities.iter().map(|i| i.as_ref()))?;
    let mut buf = String::new();
    r.read_to_string(&mut buf)?;
    let phrase = SecretString::new(buf);
    parse_mnemonic(&phrase)
}

/// Reads and decrypts the mnemonic for a wallet.
pub fn load_mnemonic(
    encrypted_path: &Path,
    identity_path: &Path,
) -> anyhow::Result<Mnemonic<English>> {
    let ct = std::fs::read(encrypted_path)
        .with_context(|| format!("reading {}", encrypted_path.display()))?;
    decrypt_mnemonic(&ct, &load_identities(identity_path)?)
}

pub fn write_private_file(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(contents)?;
    f.sync_all()?;
    restrict_permissions(path);
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {
    // On Windows the file inherits the ACL of the per-user profile directory
    // (%LOCALAPPDATA%), which is already restricted to the user. Production
    // deployments must use a KMS/HSM instead (see docs/PHASE0_REPORT.md).
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonic_round_trips_through_age() {
        let dir = tempfile::tempdir().unwrap();
        let id_path = dir.path().join("id.txt");
        let id = create_identity(&id_path).unwrap();
        let m = generate_mnemonic();
        let ct = encrypt_mnemonic(&m, &id.to_public()).unwrap();
        // Ciphertext must not contain any mnemonic word in the clear.
        let ct_str = String::from_utf8_lossy(&ct);
        for word in m.phrase().split(' ') {
            assert!(!ct_str.contains(&format!(" {word} ")));
        }
        let back = decrypt_mnemonic(&ct, &load_identities(&id_path).unwrap()).unwrap();
        assert_eq!(back.phrase(), m.phrase());
    }

    #[test]
    fn wrong_identity_cannot_decrypt() {
        let dir = tempfile::tempdir().unwrap();
        let a = create_identity(&dir.path().join("a.txt")).unwrap();
        create_identity(&dir.path().join("b.txt")).unwrap();
        let ct = encrypt_mnemonic(&generate_mnemonic(), &a.to_public()).unwrap();
        let wrong = load_identities(&dir.path().join("b.txt")).unwrap();
        assert!(decrypt_mnemonic(&ct, &wrong).is_err());
    }

    #[test]
    fn identity_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("id.txt");
        create_identity(&p).unwrap();
        assert!(create_identity(&p).is_err());
    }

    #[test]
    fn rejects_bad_checksum() {
        let bad = SecretString::new(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
             abandon abandon"
                .into(),
        );
        assert!(parse_mnemonic(&bad).is_err());
    }
}
