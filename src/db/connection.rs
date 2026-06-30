//! Postgres connection-pool setup.

use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use tracing::{debug, info};

/// Reads `DATABASE_URL` from the environment (via dotenvy / `.env`) and
/// returns a connection pool with at most `max_connections` live connections.
pub async fn setup_pool(max_connections: u32) -> Result<Pool<Postgres>, Box<dyn std::error::Error>> {
    let database_url = dotenvy::var("DATABASE_URL")?;
    debug!("connecting to postgres pool (max_connections={})", max_connections);
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(&database_url)
        .await?;
    info!("postgres pool ready");
    Ok(pool)
}
