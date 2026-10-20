use rusqlite::{Connection, params};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_ENTRIES: usize = 50;
pub const MAX_BYTES: usize = 100_000;

pub struct Db {
    conn: Connection,
}

pub struct Entry {
    pub id: i64,
    pub content: String,
    pub pinned: bool,
}

/// Microseconds since the epoch.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

fn db_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .expect("neither XDG_DATA_HOME nor HOME is set");
    base.join("diodon-next").join("history.db")
}

impl Db {
    /// Open the on-disk database, creating it with private permissions.
    pub fn open() -> rusqlite::Result<Db> {
        let path = db_path();
        let dir = path.parent().unwrap();
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let conn = Connection::open(&path)?;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Db> {
        // journal_mode returns a row, so it must be read as a query.
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS entries (
                   id         INTEGER PRIMARY KEY,
                   content    TEXT    NOT NULL UNIQUE,
                   created_at INTEGER NOT NULL,
                   used_at    INTEGER NOT NULL,
                   pinned     INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE INDEX IF NOT EXISTS entries_used_at ON entries(used_at DESC);
                 PRAGMA user_version = 1;",
            )?;
        }
        Ok(Db { conn })
    }

    /// Insert, or bump an existing identical entry to the top.
    pub fn upsert(&self, content: &str) -> rusqlite::Result<()> {
        // used_at must be strictly increasing so the newest bump always sorts first,
        // even within the same clock tick.
        self.conn.execute(
            "INSERT INTO entries (content, created_at, used_at)
             SELECT ?1, ?2, MAX(?2, COALESCE(MAX(used_at), 0) + 1) FROM entries WHERE true
             ON CONFLICT(content) DO UPDATE SET used_at = excluded.used_at",
            params![content, now()],
        )?;
        self.prune(MAX_ENTRIES)
    }

    /// All pinned entries, then the `limit` most recent unpinned ones.
    /// Pinned entries do not count against the limit.
    pub fn recent(&self, limit: usize) -> rusqlite::Result<Vec<Entry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content, pinned FROM entries
             WHERE pinned = 1 OR id IN (
               SELECT id FROM entries WHERE pinned = 0
               ORDER BY used_at DESC, id DESC LIMIT ?1)
             ORDER BY pinned DESC, used_at DESC, id DESC",
        )?;
        stmt.query_map([limit as i64], |r| {
            Ok(Entry {
                id: r.get(0)?,
                content: r.get(1)?,
                pinned: r.get::<_, i64>(2)? != 0,
            })
        })?
        .collect()
    }

    /// Delete unpinned rows beyond the newest `max`.
    pub fn prune(&self, max: usize) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM entries WHERE pinned = 0 AND id NOT IN (
               SELECT id FROM entries WHERE pinned = 0
               ORDER BY used_at DESC, id DESC LIMIT ?1)",
            [max as i64],
        )?;
        Ok(())
    }

    pub fn clear(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM entries WHERE pinned = 0", [])?;
        Ok(())
    }

    pub fn toggle_pin(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE entries SET pinned = 1 - pinned WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contents(db: &Db) -> Vec<String> {
        db.recent(100).unwrap().into_iter().map(|e| e.content).collect()
    }

    #[test]
    fn dedupes_and_moves_to_top() {
        let db = Db::open_in_memory().unwrap();
        db.upsert("a").unwrap();
        db.upsert("b").unwrap();
        db.upsert("a").unwrap();
        assert_eq!(contents(&db), vec!["a", "b"]);
    }

    #[test]
    fn newest_first() {
        let db = Db::open_in_memory().unwrap();
        db.upsert("a").unwrap();
        db.upsert("b").unwrap();
        db.upsert("c").unwrap();
        assert_eq!(contents(&db), vec!["c", "b", "a"]);
    }

    #[test]
    fn prune_keeps_newest_and_pinned() {
        let db = Db::open_in_memory().unwrap();
        db.upsert("old").unwrap();
        let id = db.recent(1).unwrap()[0].id;
        db.toggle_pin(id).unwrap();
        for i in 0..5 {
            db.upsert(&format!("e{i}")).unwrap();
        }
        db.prune(2).unwrap();
        assert_eq!(contents(&db), vec!["old", "e4", "e3"]);
    }

    #[test]
    fn pinned_flag_is_reported_and_survives_recopy() {
        let db = Db::open_in_memory().unwrap();
        db.upsert("a").unwrap();
        db.upsert("b").unwrap();
        let a = db.recent(10).unwrap().into_iter().find(|e| e.content == "a").unwrap();
        assert!(!a.pinned);
        db.toggle_pin(a.id).unwrap();
        db.upsert("a").unwrap(); // re-copy must not unpin
        let list = db.recent(10).unwrap();
        assert_eq!((list[0].content.as_str(), list[0].pinned), ("a", true));
        assert!(!list[1].pinned);
        db.toggle_pin(a.id).unwrap();
        assert!(db.recent(10).unwrap().iter().all(|e| !e.pinned));
    }

    #[test]
    fn limit_applies_to_unpinned_only() {
        let db = Db::open_in_memory().unwrap();
        db.upsert("pin").unwrap();
        let id = db.recent(1).unwrap()[0].id;
        db.toggle_pin(id).unwrap();
        for i in 0..4 {
            db.upsert(&format!("e{i}")).unwrap();
        }
        let got: Vec<String> = db.recent(2).unwrap().into_iter().map(|e| e.content).collect();
        assert_eq!(got, vec!["pin", "e3", "e2"]);
    }

    #[test]
    fn clear_keeps_pinned() {
        let db = Db::open_in_memory().unwrap();
        db.upsert("keep").unwrap();
        let id = db.recent(1).unwrap()[0].id;
        db.toggle_pin(id).unwrap();
        db.upsert("drop").unwrap();
        db.clear().unwrap();
        assert_eq!(contents(&db), vec!["keep"]);
    }
}
