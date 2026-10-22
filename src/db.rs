use gtk::glib::{Checksum, ChecksumType};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_RETAINED_ENTRIES: usize = 100;
pub const DEFAULT_DISPLAY_ENTRIES: usize = 20;
pub const DISPLAY_ENTRY_OPTIONS: [usize; 5] = [5, 10, 20, 50, 100];
pub const STORAGE_BUDGET_BYTES: i64 = 200 * 1024 * 1024;

pub struct Db {
    conn: Connection,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Formats {
    pub text: Option<String>,
    pub html: Option<Vec<u8>>,
    pub rtf: Option<Vec<u8>>,
    pub image: Option<Vec<u8>>,
    pub thumb: Option<Vec<u8>>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

impl Formats {
    pub fn size(&self) -> i64 {
        self.text.as_ref().map_or(0, |s| s.len() as i64)
            + self.html.as_ref().map_or(0, |b| b.len() as i64)
            + self.rtf.as_ref().map_or(0, |b| b.len() as i64)
            + self.image.as_ref().map_or(0, |b| b.len() as i64)
            + self.thumb.as_ref().map_or(0, |b| b.len() as i64)
    }

    pub fn hash(&self) -> String {
        let mut checksum = Checksum::new(ChecksumType::Sha256).expect("SHA-256 is available");
        for (tag, data) in [
            (1_u8, self.text.as_ref().map(|s| s.as_bytes())),
            (2, self.html.as_deref()),
            (3, self.rtf.as_deref()),
            (4, self.image.as_deref()),
        ] {
            if let Some(data) = data {
                checksum.update(&[tag]);
                checksum.update(&(data.len() as u64).to_be_bytes());
                checksum.update(data);
            }
        }
        checksum.string().expect("SHA-256 checksum")
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: i64,
    pub hash: String,
    pub content: String,
    pub pinned: bool,
    pub size: i64,
    pub has_rich: bool,
    pub is_image: bool,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub thumb: Option<Vec<u8>>,
}

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
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 2 {
            conn.execute_batch(
                "DROP TABLE IF EXISTS entries;
                 PRAGMA auto_vacuum = INCREMENTAL;
                 VACUUM;
                 CREATE TABLE entries (
                   id         INTEGER PRIMARY KEY,
                   hash       TEXT    NOT NULL UNIQUE,
                   kind       TEXT    NOT NULL DEFAULT 'text',
                   text       TEXT,
                   html       BLOB,
                   rtf        BLOB,
                   image      BLOB,
                   thumb      BLOB,
                   width      INTEGER,
                   height     INTEGER,
                   size       INTEGER NOT NULL,
                   created_at INTEGER NOT NULL,
                   used_at    INTEGER NOT NULL,
                   pinned     INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE INDEX entries_used_at ON entries(used_at DESC);
                 PRAGMA user_version = 2;",
            )?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (
               key TEXT PRIMARY KEY NOT NULL,
               value INTEGER NOT NULL
             );",
        )?;
        Ok(Db { conn })
    }

    pub fn do_not_store_passwords(&self) -> rusqlite::Result<bool> {
        let value: Option<i64> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'do_not_store_passwords'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.unwrap_or(1) != 0)
    }

    pub fn set_do_not_store_passwords(&self, enabled: bool) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES ('do_not_store_passwords', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [i64::from(enabled)],
        )?;
        Ok(())
    }

    pub fn display_entries(&self) -> rusqlite::Result<usize> {
        let value: Option<i64> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'display_entries'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| DISPLAY_ENTRY_OPTIONS.contains(value))
            .unwrap_or(DEFAULT_DISPLAY_ENTRIES))
    }

    pub fn set_display_entries(&self, count: usize) -> rusqlite::Result<()> {
        if !DISPLAY_ENTRY_OPTIONS.contains(&count) {
            return Err(rusqlite::Error::InvalidParameterName(
                "display_entries must be one of 5, 10, 20, 50, 100".to_string(),
            ));
        }
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES ('display_entries', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [count as i64],
        )?;
        Ok(())
    }

    pub fn upsert_text(&self, text: &str) -> rusqlite::Result<String> {
        let formats = Formats {
            text: Some(text.to_owned()),
            ..Formats::default()
        };
        self.upsert_formats(&formats)?;
        Ok(formats.hash())
    }

    pub fn upsert_formats(&self, formats: &Formats) -> rusqlite::Result<String> {
        self.upsert_formats_with_limits(formats, MAX_RETAINED_ENTRIES, STORAGE_BUDGET_BYTES)
    }

    fn upsert_formats_with_limits(
        &self,
        formats: &Formats,
        max_entries: usize,
        budget: i64,
    ) -> rusqlite::Result<String> {
        let hash = formats.hash();
        let timestamp = now();
        let tx = self.conn.unchecked_transaction()?;
        let used_at = next_used_at(&tx, timestamp)?;
        tx.execute(
            "INSERT INTO entries (hash, kind, text, html, rtf, image, thumb, width, height, size, created_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
             ON CONFLICT(hash) DO UPDATE SET
               kind = excluded.kind, text = excluded.text, html = excluded.html,
               rtf = excluded.rtf, image = excluded.image, thumb = excluded.thumb,
               width = excluded.width, height = excluded.height, size = excluded.size,
               used_at = excluded.used_at",
            params![
                hash,
                if formats.image.is_some() {
                    "image"
                } else {
                    "text"
                },
                formats.text,
                formats.html,
                formats.rtf,
                formats.image,
                formats.thumb,
                formats.width,
                formats.height,
                formats.size(),
                used_at,
            ],
        )?;
        prune_tx(&tx, max_entries, budget, &hash)?;
        tx.commit()?;
        self.conn.execute_batch("PRAGMA incremental_vacuum;")?;
        Ok(hash)
    }

    pub fn recent_display(&self) -> rusqlite::Result<Vec<Entry>> {
        self.recent(self.display_entries()?)
    }

    pub fn recent(&self, limit: usize) -> rusqlite::Result<Vec<Entry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, hash,
                    CASE WHEN kind = 'image' THEN printf('Image %d×%d', width, height)
                         ELSE substr(COALESCE(text, '(formatted text)'), 1, 512) END,
                    pinned, size, html IS NOT NULL OR rtf IS NOT NULL,
                    kind = 'image', width, height, thumb FROM entries
             WHERE pinned = 1 OR id IN (
               SELECT id FROM entries WHERE pinned = 0
               ORDER BY used_at DESC, id DESC LIMIT ?1)
             ORDER BY pinned DESC, used_at DESC, id DESC",
        )?;
        stmt.query_map([limit as i64], |r| {
            Ok(Entry {
                id: r.get(0)?,
                hash: r.get(1)?,
                content: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
                size: r.get(4)?,
                has_rich: r.get(5)?,
                is_image: r.get(6)?,
                width: r.get(7)?,
                height: r.get(8)?,
                thumb: r.get(9)?,
            })
        })?
        .collect()
    }

    pub fn get_formats(&self, id: i64) -> rusqlite::Result<Option<Formats>> {
        self.conn
            .query_row(
                "SELECT text, html, rtf, image, thumb, width, height FROM entries WHERE id = ?1",
                [id],
                |r| {
                    Ok(Formats {
                        text: r.get(0)?,
                        html: r.get(1)?,
                        rtf: r.get(2)?,
                        image: r.get(3)?,
                        thumb: r.get(4)?,
                        width: r.get(5)?,
                        height: r.get(6)?,
                    })
                },
            )
            .optional()
    }

    pub fn touch(&self, id: i64) -> rusqlite::Result<()> {
        let timestamp = now();
        self.conn.execute(
            "UPDATE entries SET used_at = MAX(?1, COALESCE((SELECT MAX(used_at) FROM entries), 0) + 1) WHERE id = ?2",
            params![timestamp, id],
        )?;
        Ok(())
    }

    pub fn checkpoint(&self) {
        let r: rusqlite::Result<(i64, i64, i64)> =
            self.conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                });
        if let Err(e) = r {
            eprintln!("diodon-next: checkpoint failed: {e}");
        }
    }

    pub fn clear(&self) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM entries WHERE pinned = 0", [])?;
        Ok(())
    }

    pub fn toggle_pin(&self, id: i64) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("UPDATE entries SET pinned = 1 - pinned WHERE id = ?1", [id])?;
        prune_tx(&tx, MAX_RETAINED_ENTRIES, STORAGE_BUDGET_BYTES, "")?;
        tx.commit()?;
        Ok(())
    }
}

fn next_used_at(tx: &Transaction<'_>, timestamp: i64) -> rusqlite::Result<i64> {
    tx.query_row(
        "SELECT MAX(?1, COALESCE(MAX(used_at), 0) + 1) FROM entries",
        [timestamp],
        |r| r.get(0),
    )
}

fn prune_tx(
    tx: &Transaction<'_>,
    max_entries: usize,
    budget: i64,
    newest_hash: &str,
) -> rusqlite::Result<()> {
    loop {
        let (count, size): (i64, i64) = tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM entries WHERE pinned = 0",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if count <= max_entries as i64 && size <= budget {
            break;
        }
        let oldest: Option<i64> = tx
            .query_row(
                "SELECT id FROM entries WHERE pinned = 0 AND hash != ?1
                 ORDER BY used_at ASC, id ASC LIMIT 1",
                [newest_hash],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = oldest else {
            break;
        };
        tx.execute("DELETE FROM entries WHERE id = ?1", [id])?;
    }
    tx.execute_batch("PRAGMA incremental_vacuum;")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contents(db: &Db) -> Vec<String> {
        db.recent(100)
            .unwrap()
            .into_iter()
            .map(|e| e.content)
            .collect()
    }

    #[test]
    fn hash_is_stable_and_distinguishes_formats() {
        let plain = Formats {
            text: Some("hello".into()),
            ..Formats::default()
        };
        let rich = Formats {
            text: Some("hello".into()),
            html: Some(b"<b>hello</b>".to_vec()),
            ..Formats::default()
        };
        assert_eq!(plain.hash(), plain.clone().hash());
        assert_ne!(plain.hash(), rich.hash());
        assert_ne!(
            Formats {
                text: Some("ab".into()),
                ..Formats::default()
            }
            .hash(),
            Formats {
                text: Some("a".into()),
                html: Some(b"b".to_vec()),
                ..Formats::default()
            }
            .hash()
        );
    }

    #[test]
    fn password_recording_setting_defaults_on_and_persists_changes() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.do_not_store_passwords().unwrap());
        db.set_do_not_store_passwords(false).unwrap();
        assert!(!db.do_not_store_passwords().unwrap());
        db.set_do_not_store_passwords(true).unwrap();
        assert!(db.do_not_store_passwords().unwrap());
    }

    #[test]
    fn display_entry_setting_defaults_and_changes_without_pruning() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.display_entries().unwrap(), DEFAULT_DISPLAY_ENTRIES);
        for index in 0..12 {
            db.upsert_text(&format!("entry {index}")).unwrap();
        }
        db.set_display_entries(5).unwrap();
        assert_eq!(db.display_entries().unwrap(), 5);
        assert_eq!(db.recent_display().unwrap().len(), 5);
        assert_eq!(db.recent(MAX_RETAINED_ENTRIES).unwrap().len(), 12);
        db.set_display_entries(100).unwrap();
        assert_eq!(db.display_entries().unwrap(), 100);
        assert_eq!(db.recent_display().unwrap().len(), 12);
        assert_eq!(db.recent(MAX_RETAINED_ENTRIES).unwrap().len(), 12);
        assert!(db.set_display_entries(7).is_err());
    }

    #[test]
    fn fresh_database_uses_schema_version_two() {
        let db = Db::open_in_memory().unwrap();
        let version: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        let columns: Vec<String> = db
            .conn
            .prepare("PRAGMA table_info(entries)")
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(version, 2);
        assert!(columns.contains(&"hash".to_string()));
        assert!(columns.contains(&"html".to_string()));
        assert!(columns.contains(&"image".to_string()));
        assert!(!columns.contains(&"content".to_string()));
    }

    #[test]
    fn stores_image_data_dimensions_and_thumbnail() {
        let db = Db::open_in_memory().unwrap();
        let formats = Formats {
            image: Some(b"png data".to_vec()),
            thumb: Some(b"thumb data".to_vec()),
            width: Some(800),
            height: Some(600),
            ..Formats::default()
        };
        let hash = db.upsert_formats(&formats).unwrap();
        let entry = db.recent(1).unwrap().remove(0);
        assert_eq!(entry.hash, hash);
        assert!(entry.is_image);
        assert_eq!((entry.width, entry.height), (Some(800), Some(600)));
        assert_eq!(entry.thumb, Some(b"thumb data".to_vec()));
        assert_eq!(entry.content, "Image 800×600");
        assert_eq!(db.get_formats(entry.id).unwrap(), Some(formats));
    }

    #[test]
    fn dedupes_identical_images_by_hash() {
        let db = Db::open_in_memory().unwrap();
        let image = Formats {
            image: Some(b"same png bytes".to_vec()),
            thumb: Some(b"same thumbnail".to_vec()),
            width: Some(3),
            height: Some(2),
            ..Formats::default()
        };
        db.upsert_formats(&image).unwrap();
        db.upsert_formats(&image).unwrap();
        assert_eq!(db.recent(10).unwrap().len(), 1);
    }

    #[test]
    fn stores_text_larger_than_old_per_entry_limit() {
        let db = Db::open_in_memory().unwrap();
        let text = "x".repeat(150_000);
        db.upsert_text(&text).unwrap();
        let stored: (i64, i64) = db
            .conn
            .query_row("SELECT length(text), size FROM entries", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(stored, (150_000, 150_000));
        assert_eq!(db.recent(1).unwrap()[0].content.len(), 512);
    }

    #[test]
    fn dedupes_identical_formats_and_moves_to_top() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_text("a").unwrap();
        db.upsert_text("b").unwrap();
        db.upsert_text("a").unwrap();
        assert_eq!(contents(&db), vec!["a", "b"]);
    }

    #[test]
    fn different_rich_formats_are_distinct_entries() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_formats(&Formats {
            text: Some("hello".into()),
            ..Formats::default()
        })
        .unwrap();
        db.upsert_formats(&Formats {
            text: Some("hello".into()),
            html: Some(b"<b>hello</b>".to_vec()),
            ..Formats::default()
        })
        .unwrap();
        assert_eq!(db.recent(10).unwrap().len(), 2);
    }

    #[test]
    fn newest_first_and_touch_by_id() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_text("a").unwrap();
        db.upsert_text("b").unwrap();
        db.upsert_text("c").unwrap();
        let a = db
            .recent(10)
            .unwrap()
            .into_iter()
            .find(|e| e.content == "a")
            .unwrap();
        db.touch(a.id).unwrap();
        assert_eq!(contents(&db), vec!["a", "c", "b"]);
    }

    #[test]
    fn pinned_flag_is_reported_and_survives_recopy() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_text("a").unwrap();
        db.upsert_text("b").unwrap();
        let a = db
            .recent(10)
            .unwrap()
            .into_iter()
            .find(|e| e.content == "a")
            .unwrap();
        db.toggle_pin(a.id).unwrap();
        db.upsert_text("a").unwrap();
        let list = db.recent(10).unwrap();
        assert_eq!((list[0].content.as_str(), list[0].pinned), ("a", true));
        assert!(!list[1].pinned);
    }

    #[test]
    fn retention_cap_is_fixed_at_one_hundred_entries() {
        let db = Db::open_in_memory().unwrap();
        for index in 0..MAX_RETAINED_ENTRIES + 3 {
            db.upsert_text(&format!("entry {index}")).unwrap();
        }
        assert_eq!(
            db.recent(MAX_RETAINED_ENTRIES + 10).unwrap().len(),
            MAX_RETAINED_ENTRIES
        );
    }

    #[test]
    fn count_limit_keeps_pinned_and_newest() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_text("pin").unwrap();
        let id = db.recent(1).unwrap()[0].id;
        db.toggle_pin(id).unwrap();
        for i in 0..(MAX_RETAINED_ENTRIES + 3) {
            db.upsert_text(&format!("e{i}")).unwrap();
        }
        let got = contents(&db);
        assert_eq!(got.len(), MAX_RETAINED_ENTRIES + 1);
        assert_eq!(got[0], "pin");
        assert_eq!(got[1], format!("e{}", MAX_RETAINED_ENTRIES + 2));
    }

    #[test]
    fn budget_evicts_oldest_unpinned_but_keeps_new_oversized_entry() {
        let db = Db::open_in_memory().unwrap();
        let a = Formats {
            text: Some("a".repeat(7)),
            ..Formats::default()
        };
        let b = Formats {
            text: Some("b".repeat(7)),
            ..Formats::default()
        };
        let c = Formats {
            text: Some("c".repeat(20)),
            ..Formats::default()
        };
        db.upsert_formats_with_limits(&a, MAX_RETAINED_ENTRIES, 10)
            .unwrap();
        db.upsert_formats_with_limits(&b, MAX_RETAINED_ENTRIES, 10)
            .unwrap();
        db.upsert_formats_with_limits(&c, MAX_RETAINED_ENTRIES, 10)
            .unwrap();
        let entries = db.recent(10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, c.text.unwrap());
    }

    #[test]
    fn budget_ignores_pinned_rows_and_prunes_oldest_unpinned() {
        let db = Db::open_in_memory().unwrap();
        let pinned = Formats {
            text: Some("p".repeat(50)),
            ..Formats::default()
        };
        db.upsert_formats_with_limits(&pinned, MAX_RETAINED_ENTRIES, 10)
            .unwrap();
        let id = db.recent(10).unwrap()[0].id;
        db.toggle_pin(id).unwrap();
        db.upsert_formats_with_limits(
            &Formats {
                text: Some("old".into()),
                ..Formats::default()
            },
            MAX_RETAINED_ENTRIES,
            10,
        )
        .unwrap();
        let new = Formats {
            text: Some("new".repeat(4)),
            ..Formats::default()
        };
        db.upsert_formats_with_limits(&new, MAX_RETAINED_ENTRIES, 10)
            .unwrap();
        let entries = db.recent(10).unwrap();
        assert!(entries.iter().any(|e| e.pinned));
        assert!(
            entries
                .iter()
                .any(|e| e.content == new.text.as_ref().unwrap().as_str())
        );
        assert!(!entries.iter().any(|e| e.content == "old"));
    }

    #[test]
    fn clear_keeps_pinned() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_text("keep").unwrap();
        let id = db.recent(1).unwrap()[0].id;
        db.toggle_pin(id).unwrap();
        db.upsert_text("drop").unwrap();
        db.clear().unwrap();
        assert_eq!(contents(&db), vec!["keep"]);
    }
}
