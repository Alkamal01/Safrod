use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("not found")]
    NotFound,
    #[error("already exists")]
    Conflict,
    #[error("idempotency key was already used with different parameters")]
    IdempotencyMismatch,
    #[error("illegal state transition {from} -> {to}")]
    IllegalTransition {
        from: &'static str,
        to: &'static str,
    },
    #[error("insufficient simulated balance")]
    InsufficientBalance,
    #[error("amount out of range")]
    AmountOutOfRange,
    #[error("database error: {0}")]
    Database(#[source] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

impl From<sqlx::Error> for StorageError {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            sqlx::Error::RowNotFound => StorageError::NotFound,
            sqlx::Error::Database(db) if db.is_unique_violation() => StorageError::Conflict,
            _ => StorageError::Database(err),
        }
    }
}
