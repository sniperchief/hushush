//! Filesystem layout. Wallet state and secrets live OUTSIDE the source tree by
//! default, and we refuse to put them inside a git working tree at all.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// Root for all Phase 0 wallet data. Override with `HUSHUSH_DATA_DIR`.
pub fn data_root() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("HUSHUSH_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .context("cannot determine a data directory; set HUSHUSH_DATA_DIR")?;
    Ok(base.join("hushush-phase0"))
}

/// Directory holding the age identities that decrypt each wallet's mnemonic.
/// Kept separate from wallet state so that a copy of the wallet directory alone
/// does not carry spend authority. Override with `HUSHUSH_SECRETS_DIR`.
pub fn secrets_root() -> anyhow::Result<PathBuf> {
    match std::env::var_os("HUSHUSH_SECRETS_DIR") {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(data_root()?.join("secrets")),
    }
}

#[derive(Clone, Debug)]
pub struct WalletPaths {
    pub name: String,
    pub dir: PathBuf,
    pub data_db: PathBuf,
    pub encrypted_mnemonic: PathBuf,
    pub identity: PathBuf,
}

impl WalletPaths {
    pub fn for_name(name: &str) -> anyhow::Result<Self> {
        Self::in_roots(name, &data_root()?.join("wallets"), &secrets_root()?)
    }

    pub fn in_roots(name: &str, wallets_root: &Path, secrets_root: &Path) -> anyhow::Result<Self> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("wallet name must be non-empty and contain only [A-Za-z0-9_-]");
        }
        let dir = wallets_root.join(name);
        Ok(Self {
            name: name.to_string(),
            data_db: dir.join("data.sqlite"),
            encrypted_mnemonic: dir.join("mnemonic.age"),
            identity: secrets_root.join(format!("{name}.age-identity.txt")),
            dir,
        })
    }

    pub fn exists(&self) -> bool {
        self.data_db.exists()
    }

    /// Refuses wallet/secret locations inside a git working tree, so state and
    /// key material cannot be committed by accident.
    pub fn ensure_outside_source_control(&self) -> anyhow::Result<()> {
        for p in [&self.dir, &self.identity] {
            if let Some(repo) = enclosing_git_repo(p) {
                bail!(
                    "refusing to store wallet data at {} because it is inside the git \
                     repository at {}; set HUSHUSH_DATA_DIR / HUSHUSH_SECRETS_DIR elsewhere",
                    p.display(),
                    repo.display()
                );
            }
        }
        Ok(())
    }
}

/// Returns the root of the git working tree containing `path`, if any.
pub fn enclosing_git_repo(path: &Path) -> Option<PathBuf> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    abs.ancestors()
        .find(|a| a.join(".git").exists())
        .map(Path::to_path_buf)
}
