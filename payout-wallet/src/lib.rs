//! hushush payout wallet service (Phase 3, TESTNET ONLY).
//!
//! The only hushush process that can spend. It wraps the Phase 0 wallet proof
//! (`hushush_wallet_proof`) — the code that sent and received real Ironwood
//! shielded testnet payments — behind a narrow, authenticated HTTP interface,
//! and adds what a payout system needs on top: a durable per-payment intent
//! record (one transaction per payment id, ever), crash recovery, rebroadcast,
//! a stricter recipient/pool policy, and chain-based verification.
//!
//! Keys stay where Phase 0 put them: an age-encrypted mnemonic plus a separate
//! identity file, both outside the repository, decrypted only for the moment a
//! transaction is signed. The browser, the web app and the payroll database
//! never see them; payroll-api talks to this service with its own token.

pub mod actor;
pub mod chain;
pub mod http;
pub mod intents;
pub mod policy;
mod resend;

/// Halo 2 proving recurses deeply; Windows' default 1 MiB stack overflows
/// (found in Phase 0). Every thread that may prove or decrypt gets this much.
pub const STACK_SIZE: usize = 64 * 1024 * 1024;
