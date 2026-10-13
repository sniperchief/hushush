use thiserror::Error;

#[derive(Debug, Error)]
pub enum PayrollError {
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("not authorized: {0}")]
    Forbidden(&'static str),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("invalid state: {0}")]
    InvalidState(String),
    #[error(
        "insufficient available balance: requested {requested} zats, available {available} zats"
    )]
    InsufficientAvailable { requested: i64, available: i64 },
    #[error("idempotency key already used for a different claim request")]
    IdempotencyConflict,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("server clock is behind the stream's last event; refusing to rewrite history")]
    ClockSkew,
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("database error: {0}")]
    Pg(#[from] postgres::Error),
}

pub type Result<T> = std::result::Result<T, PayrollError>;
