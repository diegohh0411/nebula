use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Folder {
    pub id: i64,
    pub path: String,
    pub added_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImageRecord {
    pub id: i64,
    pub folder_id: i64,
    pub file_path: String,
    pub file_name: String,
    pub file_size: Option<i64>,
    pub created_at: Option<String>,
    pub indexed_at: String,
    pub embedded: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexingStatus {
    pub total: i64,
    pub embedded: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: i64,
    pub file_path: String,
    pub file_name: String,
    pub similarity: f64,
}

pub fn init_db(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS folders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT NOT NULL UNIQUE,
            added_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS images (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
            file_path TEXT NOT NULL UNIQUE,
            file_name TEXT NOT NULL,
            file_size INTEGER,
            created_at TEXT,
            indexed_at TEXT NOT NULL DEFAULT (datetime('now')),
            embedded INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS embeddings (
            image_id INTEGER PRIMARY KEY REFERENCES images(id) ON DELETE CASCADE,
            embedding BLOB NOT NULL
        );",
    )?;
    Ok(())
}

pub fn add_folder(conn: &Connection, path: &str) -> SqlResult<Folder> {
    conn.execute("INSERT INTO folders (path) VALUES (?1)", params![path])?;
    let id = conn.last_insert_rowid();
    Ok(Folder {
        id,
        path: path.to_string(),
        added_at: String::new(),
    })
}

pub fn remove_folder(conn: &Connection, id: i64) -> SqlResult<()> {
    conn.execute("DELETE FROM folders WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list_folders(conn: &Connection) -> SqlResult<Vec<Folder>> {
    let mut stmt =
        conn.prepare("SELECT id, path, added_at FROM folders ORDER BY added_at DESC")?;
    let rows = stmt.query_map([], |row| {
        Ok(Folder {
            id: row.get(0)?,
            path: row.get(1)?,
            added_at: row.get(2)?,
        })
    })?;
    rows.collect()
}

pub fn add_images(
    conn: &Connection,
    folder_id: i64,
    images: &[(String, String, Option<i64>)],
) -> SqlResult<usize> {
    let mut count = 0;
    for (file_path, file_name, file_size) in images {
        match conn.execute(
            "INSERT OR IGNORE INTO images (folder_id, file_path, file_name, file_size) VALUES (?1, ?2, ?3, ?4)",
            params![folder_id, file_path, file_name, file_size],
        ) {
            Ok(_) => count += 1,
            Err(_) => continue,
        }
    }
    Ok(count)
}

pub fn remove_images_for_folder(conn: &Connection, folder_id: i64) -> SqlResult<usize> {
    Ok(conn.execute(
        "DELETE FROM images WHERE folder_id = ?1",
        params![folder_id],
    )?)
}

pub fn get_unembedded_images(conn: &Connection) -> SqlResult<Vec<ImageRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, folder_id, file_path, file_name, file_size, created_at, indexed_at, embedded
         FROM images WHERE embedded = 0",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ImageRecord {
            id: row.get(0)?,
            folder_id: row.get(1)?,
            file_path: row.get(2)?,
            file_name: row.get(3)?,
            file_size: row.get(4)?,
            created_at: row.get(5)?,
            indexed_at: row.get(6)?,
            embedded: row.get::<_, i32>(7)? != 0,
        })
    })?;
    rows.collect()
}

pub fn store_embedding(conn: &Connection, image_id: i64, embedding: &[u8]) -> SqlResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO embeddings (image_id, embedding) VALUES (?1, ?2)",
        params![image_id, embedding],
    )?;
    Ok(())
}

pub fn mark_embedded(conn: &Connection, image_id: i64) -> SqlResult<()> {
    conn.execute(
        "UPDATE images SET embedded = 1 WHERE id = ?1",
        params![image_id],
    )?;
    Ok(())
}

pub fn get_indexing_status(conn: &Connection) -> SqlResult<IndexingStatus> {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM images", [], |row| row.get(0))?;
    let embedded: i64 =
        conn.query_row("SELECT COUNT(*) FROM images WHERE embedded = 1", [], |row| {
            row.get(0)
        })?;
    Ok(IndexingStatus { total, embedded })
}

pub fn get_all_embeddings(conn: &Connection) -> SqlResult<Vec<(i64, String, String, Vec<u8>)>> {
    let mut stmt = conn.prepare(
        "SELECT i.id, i.file_path, i.file_name, e.embedding
         FROM images i JOIN embeddings e ON i.id = e.image_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Vec<u8>>(3)?,
        ))
    })?;
    rows.collect()
}

pub fn get_images_paginated(
    conn: &Connection,
    offset: i64,
    limit: i64,
) -> SqlResult<Vec<ImageRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, folder_id, file_path, file_name, file_size, created_at, indexed_at, embedded
         FROM images ORDER BY indexed_at DESC LIMIT ?1 OFFSET ?2",
    )?;
    let rows = stmt.query_map(params![limit, offset], |row| {
        Ok(ImageRecord {
            id: row.get(0)?,
            folder_id: row.get(1)?,
            file_path: row.get(2)?,
            file_name: row.get(3)?,
            file_size: row.get(4)?,
            created_at: row.get(5)?,
            indexed_at: row.get(6)?,
            embedded: row.get::<_, i32>(7)? != 0,
        })
    })?;
    rows.collect()
}

pub fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

pub fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn test_init_db() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(init_db(&conn).is_ok());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM folders", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_add_and_list_folders() {
        let conn = test_db();
        let folder = add_folder(&conn, "/tmp/photos").unwrap();
        assert_eq!(folder.path, "/tmp/photos");

        let folders = list_folders(&conn).unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].path, "/tmp/photos");
    }

    #[test]
    fn test_add_duplicate_folder_fails() {
        let conn = test_db();
        add_folder(&conn, "/tmp/photos").unwrap();
        assert!(add_folder(&conn, "/tmp/photos").is_err());
    }

    #[test]
    fn test_remove_folder_cascades() {
        let conn = test_db();
        let folder = add_folder(&conn, "/tmp/photos").unwrap();
        add_images(
            &conn,
            folder.id,
            &[
                (
                    "/tmp/photos/a.jpg".into(),
                    "a.jpg".into(),
                    Some(1024),
                ),
            ],
        )
        .unwrap();
        remove_folder(&conn, folder.id).unwrap();
        let images = get_unembedded_images(&conn).unwrap();
        assert!(images.is_empty());
    }

    #[test]
    fn test_embedding_roundtrip() {
        let original: Vec<f32> = vec![0.1, -0.2, 0.3, 1.0, -0.5];
        let bytes = embedding_to_bytes(&original);
        assert_eq!(bytes.len(), 20); // 5 floats * 4 bytes
        let restored = bytes_to_embedding(&bytes);
        for (a, b) in original.iter().zip(restored.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn test_indexing_status() {
        let conn = test_db();
        let folder = add_folder(&conn, "/tmp/photos").unwrap();
        add_images(
            &conn,
            folder.id,
            &[
                (
                    "/tmp/photos/a.jpg".into(),
                    "a.jpg".into(),
                    Some(1024),
                ),
                (
                    "/tmp/photos/b.png".into(),
                    "b.png".into(),
                    Some(2048),
                ),
            ],
        )
        .unwrap();
        let status = get_indexing_status(&conn).unwrap();
        assert_eq!(status.total, 2);
        assert_eq!(status.embedded, 0);
    }
}
