use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use tracing::{debug, info};

// Connection setup. Reads DATABASE_URL from the environment (.env via dotenvy)
// and hands back a ready Postgres pool.
pub async fn setup_pool(c: u32) -> Result<Pool<Postgres>, Box<dyn std::error::Error>> {
    let database_url = dotenvy::var("DATABASE_URL")?;
    debug!("connecting to postgres pool (max_connections={})", c);
    let pool = PgPoolOptions::new()
        .max_connections(c)
        .connect(&database_url)
        .await?;
    info!("postgres pool ready");
    Ok(pool)
}
