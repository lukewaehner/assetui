//! Postgres connection-pool setup.

use std::time::Duration;

use sqlx::{Pool, Postgres, postgres::PgConnectOptions, postgres::PgPoolOptions};
use tracing::{debug, info};

use crate::AppError;

/// Default pool size shared by the CLI and TUI binaries.
pub const DEFAULT_MAX_CONNECTIONS: u32 = 5;

/// Server-side cap on how long any single statement may run.
const STATEMENT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to wait for a lock before giving up.
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `acquire()` waits for a free pooled connection.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);

/// Returns a connection pool with at most `max_connections` live connections.
pub async fn setup_pool(
    database_url: &str,
    max_connections: u32,
) -> Result<Pool<Postgres>, AppError> {
    debug!(
        "connecting to postgres pool (max_connections={})",
        max_connections
    );

    let options: PgConnectOptions = database_url.parse::<PgConnectOptions>()?.options([
        ("statement_timeout", STATEMENT_TIMEOUT.as_millis()),
        ("lock_timeout", LOCK_TIMEOUT.as_millis()),
    ]);

    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(1)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect_with(options)
        .await?;
    info!("postgres pool ready");
    Ok(pool)
}
