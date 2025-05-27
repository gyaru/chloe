use tokio_postgres::{Client, Error as PgError};
use tracing::{info, error};

pub struct Database {
    client: Client,
}

impl Database {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn test_connection(&self) -> Result<(), PgError> {
        let rows = self.client.query("SELECT 1", &[]).await?;
        info!("Database connection test successful, returned {} rows", rows.len());
        Ok(())
    }

    // Add more database operations here as needed
}