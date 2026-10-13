//! hushush Phase 0: shielded payment feasibility proof.
//!
//! A deliberately small wallet built directly on the official Zcash Rust wallet
//! stack (`zcash_client_backend` + `zcash_client_sqlite` from librustzcash).
//! No cryptography is implemented here: key derivation, note encryption, proof
//! generation, transaction construction and seed encryption are all delegated to
//! the upstream crates (and to `age` for seed-at-rest encryption).
//!
//! This crate is TESTNET ONLY. The network is fixed at compile time; there is no
//! code path that can construct a mainnet wallet.

pub mod blockcache;
pub mod config;
pub mod observer;
pub mod payment;
pub mod remote;
pub mod secrets;
pub mod sync;
pub mod wallet;

/// The only network this proof will operate on.
pub const NETWORK: zcash_protocol::consensus::Network =
    zcash_protocol::consensus::Network::TestNetwork;
