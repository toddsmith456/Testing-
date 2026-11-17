//! Local cache for *public signed events* and user-selected configuration.
//! Private keys, messages, wallet secrets and signer tokens must not be stored here.
use crate::{
    core::{Post, Profile, profile_from_json},
    publish,
};
use nostr_sdk::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct QueuedPost {
    pub id: String,
    pub event_json: String,
    pub status: String,
    pub details: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedAccount {
    pub pubkey: String,
    pub npub: String,
    pub relays: String,
}

pub struct Store {
    conn: Connection,
}
impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }
    fn init(conn: Connection) -> rusqlite::Result<Self> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 4 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        conn.execute_batch("PRAGMA foreign_keys=ON; BEGIN;
            CREATE TABLE IF NOT EXISTS config(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS feed(account TEXT NOT NULL, event_id TEXT NOT NULL, event_json TEXT NOT NULL, PRIMARY KEY(account,event_id));
            CREATE TABLE IF NOT EXISTS bookmarks(account TEXT NOT NULL, event_id TEXT NOT NULL, event_json TEXT NOT NULL, PRIMARY KEY(account,event_id));
            CREATE TABLE IF NOT EXISTS outbox(account TEXT NOT NULL, event_id TEXT NOT NULL, event_json TEXT NOT NULL, status TEXT NOT NULL, details TEXT NOT NULL DEFAULT '', PRIMARY KEY(account,event_id));
            CREATE TABLE IF NOT EXISTS profiles(account TEXT NOT NULL, author TEXT NOT NULL, event_json TEXT NOT NULL, PRIMARY KEY(account,author));
            CREATE TABLE IF NOT EXISTS accounts(pubkey TEXT PRIMARY KEY, npub TEXT NOT NULL, relays TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS local_mutes(account TEXT NOT NULL, author TEXT NOT NULL, PRIMARY KEY(account,author));")?;
        // Migrate the prior active account in the same transaction as the schema version.
        if version < 4 {
            let old_key: Option<String> = conn
                .query_row("SELECT value FROM config WHERE key='account'", [], |r| {
                    r.get(0)
                })
                .optional()?;
            let old_relays: Option<String> = conn
                .query_row("SELECT value FROM config WHERE key='relays'", [], |r| {
                    r.get(0)
                })
                .optional()?;
            if let (Some(raw), Some(relays)) = (old_key, old_relays) {
                if let Ok(key) = PublicKey::parse(&raw) {
                    if !raw.to_ascii_lowercase().starts_with("nsec") {
                        let npub = key.to_bech32().map_err(|_| rusqlite::Error::InvalidQuery)?;
                        conn.execute(
                            "INSERT OR IGNORE INTO accounts VALUES(?1,?2,?3)",
                            params![key.to_hex(), npub, relays],
                        )?;
                    }
                }
            }
        }
        conn.execute_batch("PRAGMA user_version=4; COMMIT;")?;
        Ok(Self { conn })
    }
    pub fn read_config(&self) -> rusqlite::Result<Option<(String, String)>> {
        let key: Option<String> = self
            .conn
            .query_row("SELECT value FROM config WHERE key='account'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let relays: Option<String> = self
            .conn
            .query_row("SELECT value FROM config WHERE key='relays'", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(key.zip(relays))
    }
    pub fn save_config(&mut self, account: &str, relays: &str) -> rusqlite::Result<()> {
        let t = self.conn.transaction()?;
        t.execute(
            "INSERT OR REPLACE INTO config VALUES('account',?1)",
            [account],
        )?;
        t.execute(
            "INSERT OR REPLACE INTO config VALUES('relays',?1)",
            [relays],
        )?;
        t.commit()
    }
    pub fn list_accounts(&self) -> rusqlite::Result<Vec<SavedAccount>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pubkey,npub,relays FROM accounts ORDER BY pubkey")?;
        let rows = stmt.query_map([], |r| {
            Ok(SavedAccount {
                pubkey: r.get(0)?,
                npub: r.get(1)?,
                relays: r.get(2)?,
            })
        })?;
        rows.collect()
    }
    /// Save public identifiers and select locally. The caller decides if/when to connect.
    pub fn save_account(&mut self, key: PublicKey, relays: &str) -> rusqlite::Result<()> {
        let npub = key.to_bech32().map_err(|_| rusqlite::Error::InvalidQuery)?;
        let t = self.conn.transaction()?;
        t.execute(
            "INSERT OR REPLACE INTO accounts(pubkey,npub,relays) VALUES(?1,?2,?3)",
            params![key.to_hex(), npub, relays],
        )?;
        t.execute(
            "INSERT OR REPLACE INTO config VALUES('account',?1)",
            [&npub],
        )?;
        t.execute(
            "INSERT OR REPLACE INTO config VALUES('relays',?1)",
            [relays],
        )?;
        t.commit()
    }
    pub fn select_account(&mut self, pubkey: &str) -> rusqlite::Result<SavedAccount> {
        let t = self.conn.transaction()?;
        let account: SavedAccount = t.query_row(
            "SELECT pubkey,npub,relays FROM accounts WHERE pubkey=?1",
            [pubkey],
            |r| {
                Ok(SavedAccount {
                    pubkey: r.get(0)?,
                    npub: r.get(1)?,
                    relays: r.get(2)?,
                })
            },
        )?;
        t.execute(
            "INSERT OR REPLACE INTO config VALUES('account',?1)",
            [&account.npub],
        )?;
        t.execute(
            "INSERT OR REPLACE INTO config VALUES('relays',?1)",
            [&account.relays],
        )?;
        t.commit()?;
        Ok(account)
    }
    /// Local-only mute. This never creates or publishes a Nostr event.
    pub fn set_local_mute(
        &mut self,
        account: &str,
        author: &str,
        muted: bool,
    ) -> rusqlite::Result<()> {
        let account = PublicKey::parse(account)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .to_hex();
        let author = PublicKey::parse(author)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .to_hex();
        if muted {
            self.conn.execute(
                "INSERT OR IGNORE INTO local_mutes VALUES(?1,?2)",
                params![account, author],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM local_mutes WHERE account=?1 AND author=?2",
                params![account, author],
            )?;
        }
        Ok(())
    }
    pub fn local_mutes(&self, account: &str) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT author FROM local_mutes WHERE account=?1 ORDER BY author")?;
        let rows = stmt.query_map([account], |r| r.get(0))?;
        rows.collect()
    }
    pub fn replace_feed(&mut self, account: &str, posts: &[Post]) -> rusqlite::Result<()> {
        // Keep older verified cache when a relay gives partial or empty results, bounded to 200.
        let mut merged = self.cached_feed(account)?;
        for post in posts {
            if let Some(valid) = post_from_json(&post.event_json) {
                if !merged.iter().any(|p| p.id == valid.id) {
                    merged.push(valid);
                }
            }
        }
        merged.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        merged.truncate(200);
        let t = self.conn.transaction()?;
        t.execute("DELETE FROM feed WHERE account=?1", [account])?;
        for p in &merged {
            if let Some(verified) = post_from_json(&p.event_json) {
                t.execute(
                    "INSERT OR IGNORE INTO feed VALUES(?1,?2,?3)",
                    params![account, verified.id, p.event_json],
                )?;
            }
        }
        t.commit()
    }
    pub fn profiles(&self, account: &str) -> rusqlite::Result<Vec<Profile>> {
        let mut stmt = self
            .conn
            .prepare("SELECT event_json FROM profiles WHERE account=?1")?;
        let rows = stmt.query_map([account], |r| r.get::<_, String>(0))?;
        Ok(rows
            .filter_map(|r| r.ok().and_then(|v| profile_from_json(&v)))
            .collect())
    }
    pub fn merge_profiles(&mut self, account: &str, profiles: &[Profile]) -> rusqlite::Result<()> {
        let old = self.profiles(account)?;
        let mut selected: std::collections::BTreeMap<String, Profile> =
            old.into_iter().map(|p| (p.author.clone(), p)).collect();
        for p in profiles {
            if let Some(valid) = profile_from_json(&p.event_json) {
                let replace = selected
                    .get(&valid.author)
                    .map(|old| {
                        valid.created_at > old.created_at
                            || (valid.created_at == old.created_at && valid.event_id < old.event_id)
                    })
                    .unwrap_or(true);
                if replace {
                    selected.insert(valid.author.clone(), valid);
                }
            }
        }
        let tx = self.conn.transaction()?;
        for p in selected.values().take(2000) {
            tx.execute(
                "INSERT OR REPLACE INTO profiles VALUES(?1,?2,?3)",
                params![account, p.author, p.event_json],
            )?;
        }
        tx.commit()
    }
    pub fn enqueue_signed(&mut self, account: &str, event_json: &str) -> rusqlite::Result<String> {
        let key = PublicKey::parse(account).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let event =
            publish::checked_event(key, event_json).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let id = event.id.to_hex();
        self.conn.execute("INSERT OR IGNORE INTO outbox(account,event_id,event_json,status) VALUES(?1,?2,?3,'queued')", params![account,id,event_json])?;
        Ok(id)
    }
    pub fn outbox(&self, account: &str) -> rusqlite::Result<Vec<QueuedPost>> {
        let key = PublicKey::parse(account).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let mut stmt=self.conn.prepare("SELECT event_id,event_json,status,details FROM outbox WHERE account=?1 ORDER BY rowid DESC")?;
        let rows = stmt.query_map([account], |row| {
            Ok(QueuedPost {
                id: row.get(0)?,
                event_json: row.get(1)?,
                status: row.get(2)?,
                details: row.get(3)?,
            })
        })?;
        Ok(rows
            .filter_map(|row| row.ok())
            .filter(|item| {
                publish::checked_event(key, &item.event_json)
                    .map(|e| e.id.to_hex() == item.id)
                    .unwrap_or(false)
            })
            .collect())
    }
    pub fn update_outbox(
        &mut self,
        account: &str,
        id: &str,
        status: &str,
        details: &str,
    ) -> rusqlite::Result<()> {
        let n = self.conn.execute(
            "UPDATE outbox SET status=?1,details=?2 WHERE account=?3 AND event_id=?4",
            params![status, details, account, id],
        )?;
        if n != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    }
    pub fn delete_outbox(&mut self, account: &str, id: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM outbox WHERE account=?1 AND event_id=?2",
            params![account, id],
        )?;
        Ok(())
    }
    pub fn cached_feed(&self, account: &str) -> rusqlite::Result<Vec<Post>> {
        self.read_events("SELECT event_json FROM feed WHERE account=?1", account)
    }
    pub fn bookmarks(&self, account: &str) -> rusqlite::Result<Vec<Post>> {
        self.read_events("SELECT event_json FROM bookmarks WHERE account=?1", account)
    }
    fn read_events(&self, sql: &str, account: &str) -> rusqlite::Result<Vec<Post>> {
        let mut stmt = self.conn.prepare(sql)?;
        let records = stmt.query_map([account], |r| r.get::<_, String>(0))?;
        let mut posts: Vec<Post> = records
            .filter_map(|r| r.ok().and_then(|j| post_from_json(&j)))
            .collect();
        posts.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(posts)
    }
    pub fn set_bookmark(
        &mut self,
        account: &str,
        post: &Post,
        enabled: bool,
    ) -> rusqlite::Result<()> {
        if enabled {
            let verified = post_from_json(&post.event_json).ok_or(rusqlite::Error::InvalidQuery)?;
            if verified.id != post.id {
                return Err(rusqlite::Error::InvalidQuery);
            }
            self.conn.execute(
                "INSERT OR IGNORE INTO bookmarks VALUES(?1,?2,?3)",
                params![account, post.id, post.event_json],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM bookmarks WHERE account=?1 AND event_id=?2",
                params![account, post.id],
            )?;
        }
        Ok(())
    }
}

/// A cache is not a trust boundary: parse and cryptographically reverify on every read.
fn post_from_json(json: &str) -> Option<Post> {
    if json.len() > 160_000 {
        return None;
    }
    let e = Event::from_json(json).ok()?;
    if e.kind != Kind::TextNote || e.content.len() > 64 * 1024 || e.verify().is_err() {
        return None;
    }
    Some(Post {
        id: e.id.to_hex(),
        author: e.pubkey.to_hex(),
        created_at: e.created_at.as_secs(),
        content: e.content.clone(),
        event_json: json.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid_post() -> Post {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .finalize(&keys)
            .unwrap();
        post_from_json(&event.as_json()).unwrap()
    }
    #[test]
    fn cache_and_bookmarks_are_account_scoped_and_verified() {
        let mut db = Store::init(Connection::open_in_memory().unwrap()).unwrap();
        let p = valid_post();
        db.save_config("a", "wss://relay.example").unwrap();
        assert_eq!(
            db.read_config().unwrap(),
            Some(("a".into(), "wss://relay.example".into()))
        );
        db.replace_feed("a", &[p.clone()]).unwrap();
        assert_eq!(db.cached_feed("a").unwrap().len(), 1);
        assert!(db.cached_feed("b").unwrap().is_empty());
        db.set_bookmark("a", &p, true).unwrap();
        assert_eq!(db.bookmarks("a").unwrap().len(), 1);
        assert!(db.bookmarks("b").unwrap().is_empty());
        db.set_bookmark("a", &p, false).unwrap();
        assert!(db.bookmarks("a").unwrap().is_empty());
    }
    #[test]
    fn modified_events_are_rejected_on_read_and_write() {
        let mut db = Store::init(Connection::open_in_memory().unwrap()).unwrap();
        let mut p = valid_post();
        p.event_json = p.event_json.replace("hello", "tampered");
        assert!(db.set_bookmark("a", &p, true).is_err());
        db.replace_feed("a", &[p]).unwrap();
        assert!(db.cached_feed("a").unwrap().is_empty());
    }
    #[test]
    fn outbox_validates_signed_events_and_scopes_accounts() {
        let mut db = Store::init(Connection::open_in_memory().unwrap()).unwrap();
        let owner = Keys::generate();
        let other = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "queued")
            .finalize(&owner)
            .unwrap();
        let account = owner.public_key().to_hex();
        let second = other.public_key().to_hex();
        assert!(db.enqueue_signed(&second, &event.as_json()).is_err());
        let id = db.enqueue_signed(&account, &event.as_json()).unwrap();
        assert_eq!(db.outbox(&account).unwrap().len(), 1);
        assert!(db.outbox(&second).unwrap().is_empty());
        db.update_outbox(&account, &id, "sending", "May already have been sent")
            .unwrap();
        assert_eq!(db.outbox(&account).unwrap()[0].status, "sending");
        db.delete_outbox(&account, &id).unwrap();
        assert!(db.outbox(&account).unwrap().is_empty());
    }
    #[test]
    fn empty_or_partial_refresh_preserves_cached_posts() {
        let mut db = Store::init(Connection::open_in_memory().unwrap()).unwrap();
        let first = valid_post();
        let second = valid_post();
        db.replace_feed("a", &[first.clone()]).unwrap();
        db.replace_feed("a", &[]).unwrap();
        assert_eq!(db.cached_feed("a").unwrap().len(), 1);
        db.replace_feed("a", &[second]).unwrap();
        assert_eq!(db.cached_feed("a").unwrap().len(), 2);
    }
    #[test]
    fn profile_cache_rejects_tampering_and_keeps_latest() {
        let mut db = Store::init(Connection::open_in_memory().unwrap()).unwrap();
        let key = Keys::generate();
        let e = EventBuilder::new(Kind::Metadata, r#"{"name":"Alice","about":"Bio"}"#)
            .finalize(&key)
            .unwrap();
        let profile = profile_from_json(&e.as_json()).unwrap();
        db.merge_profiles("a", &[profile.clone()]).unwrap();
        assert_eq!(db.profiles("a").unwrap()[0].name, "Alice");
        assert!(db.profiles("b").unwrap().is_empty());
        assert!(profile_from_json(&e.as_json().replace("Alice", "Eve")).is_none());
    }
    #[test]
    fn migrates_existing_database_without_losing_bookmarks() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE config(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE feed(account TEXT NOT NULL,event_id TEXT NOT NULL,event_json TEXT NOT NULL,PRIMARY KEY(account,event_id)); CREATE TABLE bookmarks(account TEXT NOT NULL,event_id TEXT NOT NULL,event_json TEXT NOT NULL,PRIMARY KEY(account,event_id)); PRAGMA user_version=1;").unwrap();
        let post = valid_post();
        conn.execute(
            "INSERT INTO bookmarks VALUES(?1,?2,?3)",
            params!["a", post.id, post.event_json],
        )
        .unwrap();
        let db = Store::init(conn).unwrap();
        assert_eq!(db.bookmarks("a").unwrap().len(), 1);
        assert_eq!(
            db.conn
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            4
        );
    }
    #[test]
    fn refuses_future_database_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(Store::init(conn).is_err());
    }
    #[test]
    fn migrates_existing_active_account_and_preserves_relay_config() {
        let conn = Connection::open_in_memory().unwrap();
        let key = Keys::generate();
        conn.execute_batch(
            "CREATE TABLE config(key TEXT PRIMARY KEY,value TEXT NOT NULL); PRAGMA user_version=3;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO config VALUES('account',?1)",
            [key.public_key().to_hex()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO config VALUES('relays',?1)",
            ["wss://private.example/path"],
        )
        .unwrap();
        let db = Store::init(conn).unwrap();
        let saved = db.list_accounts().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].pubkey, key.public_key().to_hex());
        assert_eq!(saved[0].relays, "wss://private.example/path");
        assert!(saved[0].npub.starts_with("npub1"));
    }
    #[test]
    fn accounts_and_local_mutes_are_isolated_and_reversible() {
        let mut db = Store::init(Connection::open_in_memory().unwrap()).unwrap();
        let a = Keys::generate().public_key();
        let b = Keys::generate().public_key();
        let author = Keys::generate().public_key();
        db.save_account(a, "wss://a.example").unwrap();
        db.save_account(b, "wss://b.example").unwrap();
        assert_eq!(db.list_accounts().unwrap().len(), 2);
        let selected = db.select_account(&a.to_hex()).unwrap();
        assert_eq!(selected.relays, "wss://a.example");
        assert_eq!(db.read_config().unwrap().unwrap().1, "wss://a.example");
        db.set_local_mute(&a.to_hex(), &author.to_hex(), true)
            .unwrap();
        assert_eq!(db.local_mutes(&a.to_hex()).unwrap(), vec![author.to_hex()]);
        assert!(db.local_mutes(&b.to_hex()).unwrap().is_empty());
        db.set_local_mute(&a.to_hex(), &author.to_hex(), false)
            .unwrap();
        assert!(db.local_mutes(&a.to_hex()).unwrap().is_empty());
        assert!(db.select_account("not-a-saved-account").is_err());
    }
}
