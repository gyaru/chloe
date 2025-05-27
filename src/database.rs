use sqlx::{PgPool, Row};
use tracing::info;

pub struct Database {
    pool: PgPool,
}

impl Database {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn test_connection(&self) -> Result<(), sqlx::Error> {
        let row = sqlx::query("SELECT 1 as test")
            .fetch_one(&self.pool)
            .await?;
        let test_value: i32 = row.get("test");
        info!(
            "Database connection test successful, returned: {}",
            test_value
        );
        Ok(())
    }
}
