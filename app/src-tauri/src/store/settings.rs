//! Small JSON key/value settings in SQLite.

use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub fn get<T: DeserializeOwned>(conn: &Connection, key: &str) -> rusqlite::Result<Option<T>> {
    let raw: Option<String> =
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0)).optional()?;
    Ok(raw.and_then(|text| serde_json::from_str(&text).ok()))
}

pub fn set<T: Serialize>(conn: &Connection, key: &str, value: &T) -> rusqlite::Result<()> {
    let text = serde_json::to_string(value).expect("settings values always serialize");
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, text],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_overwrites() {
        let conn = crate::store::db::open_in_memory().unwrap();
        assert_eq!(get::<u32>(&conn, "x").unwrap(), None);
        set(&conn, "x", &1u32).unwrap();
        set(&conn, "x", &2u32).unwrap();
        assert_eq!(get::<u32>(&conn, "x").unwrap(), Some(2));
    }
}
