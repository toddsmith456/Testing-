//! Local cache for *public signed events* and user-selected configuration.
//! Private keys, messages, wallet secrets and signer tokens must not be stored here.
use crate::core::Post;
use nostr_sdk::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub struct Store {
    conn: Connection,
}
impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }
    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS config(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS feed(account TEXT NOT NULL, event_id TEXT NOT NULL, event_json TEXT NOT NULL, PRIMARY KEY(account,event_id));
            CREATE TABLE IF NOT EXISTS bookmarks(account TEXT NOT NULL, event_id TEXT NOT NULL, event_json TEXT NOT NULL, PRIMARY KEY(account,event_id));
            PRAGMA user_version=1;")?;
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
    pub fn replace_feed(&mut self, account: &str, posts: &[Post]) -> rusqlite::Result<()> {
        let t = self.conn.transaction()?;
        t.execute("DELETE FROM feed WHERE account=?1", [account])?;
        for p in posts.iter().take(200) {
            if let Some(verified) = post_from_json(&p.event_json) {
                t.execute(
                    "INSERT OR IGNORE INTO feed VALUES(?1,?2,?3)",
                    params![account, verified.id, p.event_json],
                )?;
            }
        }
        t.commit()
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
}
