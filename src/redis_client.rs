use redis::{Client, Connection, RedisResult};

pub struct RedisManager {
    client: Client,
}

impl RedisManager {
    pub fn new(redis_url: &str) -> RedisResult<Self> {
        let client = Client::open(redis_url)?;
        Ok(RedisManager { client })
    }

    pub fn get_connection(&self) -> RedisResult<Connection> {
        self.client.get_connection()
    }
}
