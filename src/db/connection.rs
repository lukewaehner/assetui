//! Postgres connection-pool setup.

use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use tracing::{debug, info};

use crate::AppError;

pub async fn setup_pool(database_url: &str, max_connections: u32) -> Result<Pool<Postgres>, AppError> {
    debug!("connecting to postgres pool (max_connections={})", max_connections);
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await?;
    info!("postgres pool ready");
    Ok(pool)
}
