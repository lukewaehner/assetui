//! Pool configuration, verified against a real server.

use assetui::db::connection::{DEFAULT_MAX_CONNECTIONS, setup_pool};

#[sqlx::test]
async fn setup_pool_applies_server_side_timeouts(_pool: sqlx::PgPool) {
    // arrange
    let url = std::env::var("DATABASE_URL").expect("sqlx::test implies DATABASE_URL is set");

    // act
    let pool = setup_pool(&url, DEFAULT_MAX_CONNECTIONS)
        .await
        .expect("setup_pool should connect");
    let (statement_timeout,): (String,) = sqlx::query_as("SHOW statement_timeout")
        .fetch_one(&pool)
        .await
        .expect("SHOW statement_timeout failed");
    let (lock_timeout,): (String,) = sqlx::query_as("SHOW lock_timeout")
        .fetch_one(&pool)
        .await
        .expect("SHOW lock_timeout failed");

    // assert
    assert_eq!(statement_timeout, "30s");
    assert_eq!(lock_timeout, "5s");
}
