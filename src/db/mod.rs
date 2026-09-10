use anyhow::Result;
use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new() -> Result<Self> {
        let db_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("gitpulse");
        
        fs::create_dir_all(&db_dir)?;
        
        let db_path = db_dir.join("state.db");
        let conn = Connection::open(db_path)?;
        
        Ok(Self { conn })
    }
    
    pub fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS repos (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                last_scan TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            
            CREATE TABLE IF NOT EXISTS notifications (
                id INTEGER PRIMARY KEY,
                repo_id INTEGER,
                type TEXT NOT NULL,
                title TEXT NOT NULL,
                read BOOLEAN DEFAULT FALSE,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (repo_id) REFERENCES repos(id)
            );"
        )?;
        
        Ok(())
    }
}
