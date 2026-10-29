pub mod error;
pub mod events;
pub mod invoices;
pub mod models;
pub mod onchain;
pub mod payments;
pub mod projects;
pub mod sandbox;
pub mod webhooks;

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

pub use error::StorageError;

#[derive(Clone)]
pub struct Db {
    pool: PgPool,
}

impl Db {
    pub async fn connect(url: &str) -> Result<Self, StorageError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(Duration::from_secs(10))
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<(), StorageError> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    pub async fn ping(&self) -> Result<(), StorageError> {
        sqlx::query("select 1").execute(&self.pool).await?;
        Ok(())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// Amounts are stored as `bigint`. `MAX_SATS` in millisatoshis is far below
/// `i64::MAX`, so a failure here means a caller bypassed amount validation.
pub(crate) fn to_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::AmountOutOfRange)
}
