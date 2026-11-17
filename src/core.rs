//! Privacy-first read-only feed primitives. Nothing connects until `fetch_following` is called.
use ::url::Url;
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, time::Duration};
use thiserror::Error;

const MAX_RELAYS: usize = 8;
const MAX_FOLLOWS: usize = 2000;
const MAX_POSTS: usize = 200;
const MAX_CONTENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum FeedError {
    #[error("Enter a valid npub or public-key hex value (never an nsec)")]
    InvalidPublicKey,
    #[error("Enter at least one relay and at most {MAX_RELAYS}")]
    RelayCount,
    #[error("Relay URLs must be wss:// without credentials, query or fragment")]
    InvalidRelay,
    #[error("Nostr operation failed: {0}")]
    Network(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Post {
    pub id: String,
    pub author: String,
    pub created_at: u64,
    pub content: String,
    pub event_json: String,
}

#[derive(Clone, Debug)]
pub struct FeedResult {
    pub posts: Vec<Post>,
    pub follow_count: usize,
    pub follow_list_found: bool,
    pub truncated_follows: bool,
}

pub fn parse_public_key(value: &str) -> Result<PublicKey, FeedError> {
    let value = value.trim();
    if value.to_ascii_lowercase().starts_with("nsec") {
        return Err(FeedError::InvalidPublicKey);
    }
    PublicKey::parse(value).map_err(|_| FeedError::InvalidPublicKey)
}

pub fn parse_relays(text: &str) -> Result<Vec<String>, FeedError> {
    let mut urls = BTreeSet::new();
    for line in text.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let url = Url::parse(line).map_err(|_| FeedError::InvalidRelay)?;
        if url.scheme() != "wss"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(FeedError::InvalidRelay);
        }
        urls.insert(url.to_string());
    }
    if urls.is_empty() || urls.len() > MAX_RELAYS {
        return Err(FeedError::RelayCount);
    }
    Ok(urls.into_iter().collect())
}

/// Uses only the explicitly passed relays. A missing list is not treated as an empty authoritative list.
/// An SDK result can still be partial if some relays fail; caller must tell the user.
pub async fn fetch_following(
    pubkey: PublicKey,
    relays: Vec<String>,
) -> Result<FeedResult, FeedError> {
    let relays = parse_relays(&relays.join("\n"))?;
    fetch_following_inner(pubkey, relays).await
}

// Private helper allows tests to inject a loopback mock relay. The public entry point requires WSS.
async fn fetch_following_inner(
    pubkey: PublicKey,
    relays: Vec<String>,
) -> Result<FeedResult, FeedError> {
    let client = Client::default();
    for relay in &relays {
        client
            .add_relay(relay.as_str())
            .await
            .map_err(|e| FeedError::Network(e.to_string()))?;
    }
    client.connect().and_wait(Duration::from_secs(8)).await;
    let connections = client.relays().await;
    if !connections
        .values()
        .any(|relay| relay.status().is_connected())
    {
        client.disconnect().await;
        return Err(FeedError::Network(
            "Could not connect to any selected relay. Check the relay and your connection.".into(),
        ));
    }
    let filter = Filter::new()
        .author(pubkey)
        .kind(Kind::ContactList)
        .limit(10);
    let lists = client
        .fetch_events(filter)
        .timeout(Duration::from_secs(8))
        .await
        .map_err(|e| FeedError::Network(e.to_string()))?;
    let newest = lists
        .into_iter()
        .filter(|e| e.pubkey == pubkey && e.kind == Kind::ContactList && e.verify().is_ok())
        .max_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
    let Some(list) = newest else {
        client.disconnect().await;
        return Ok(FeedResult {
            posts: Vec::new(),
            follow_count: 0,
            follow_list_found: false,
            truncated_follows: false,
        });
    };
    let mut authors = BTreeSet::new();
    for tag in list.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) == Some("p") {
            if let Some(key) = parts.get(1).and_then(|p| PublicKey::parse(p).ok()) {
                authors.insert(key);
            }
        }
    }
    let follow_count = authors.len();
    let truncated_follows = follow_count > MAX_FOLLOWS;
    let mut selected: Vec<_> = authors.into_iter().take(MAX_FOLLOWS).collect();
    if !selected.contains(&pubkey) {
        selected.push(pubkey);
    }
    let allowed: BTreeSet<PublicKey> = selected.iter().copied().collect();
    let filter = Filter::new()
        .authors(selected)
        .kind(Kind::TextNote)
        .limit(MAX_POSTS);
    let events = client
        .fetch_events(filter)
        .timeout(Duration::from_secs(8))
        .await
        .map_err(|e| FeedError::Network(e.to_string()))?;
    let mut posts: Vec<_> = events
        .into_iter()
        .filter(|e| {
            allowed.contains(&e.pubkey)
                && e.kind == Kind::TextNote
                && e.content.len() <= MAX_CONTENT_BYTES
                && e.verify().is_ok()
        })
        .map(|e| Post {
            event_json: e.as_json(),
            id: e.id.to_hex(),
            author: e.pubkey.to_hex(),
            created_at: e.created_at.as_secs(),
            content: e.content,
        })
        .collect();
    posts.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    posts.truncate(MAX_POSTS);
    client.disconnect().await;
    Ok(FeedResult {
        posts,
        follow_count,
        follow_list_found: true,
        truncated_follows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relay_validation_is_strict_and_deduplicated() {
        let result = parse_relays(" wss://relay.example/path\nwss://relay.example/path\n").unwrap();
        assert_eq!(result, vec!["wss://relay.example/path"]);
        for invalid in [
            "ws://relay.example",
            "wss://alice:pw@relay.example",
            "wss://relay.example/?token=x",
            "wss://relay.example/#x",
        ] {
            assert!(parse_relays(invalid).is_err(), "{invalid}");
        }
        assert!(parse_relays("").is_err());
    }
    #[test]
    fn public_key_input_rejects_secrets_and_invalid_data() {
        assert!(parse_public_key("nsec1thisisnotaccepted").is_err());
        assert!(parse_public_key("not a key").is_err());
        assert!(
            parse_public_key("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .is_ok()
        );
    }
    #[test]
    fn post_round_trips_without_secret_material() {
        let post = Post {
            id: "id".into(),
            author: "author".into(),
            created_at: 1,
            content: "hello".into(),
            event_json: "{}".into(),
        };
        let data = serde_json::to_string(&post).unwrap();
        assert_eq!(serde_json::from_str::<Post>(&data).unwrap(), post);
    }
    #[tokio::test]
    async fn fetches_verified_following_notes_from_a_local_mock_relay() {
        let relay = MockRelay::run().await.unwrap();
        let url = relay.url().await.to_string();
        let publisher = Client::default();
        publisher.add_relay(url.as_str()).await.unwrap();
        publisher.connect().and_wait(Duration::from_secs(4)).await;
        assert!(
            publisher
                .relays()
                .await
                .values()
                .any(|r| r.status().is_connected())
        );
        let owner = Keys::generate();
        let followed = Keys::generate();
        let stranger = Keys::generate();
        let follows = EventBuilder::new(Kind::ContactList, "")
            .tags([Tag::public_key(followed.public_key())])
            .finalize(&owner)
            .unwrap();
        let wanted = EventBuilder::new(Kind::TextNote, "A followed person's note")
            .finalize(&followed)
            .unwrap();
        let unwanted = EventBuilder::new(Kind::TextNote, "A stranger's note")
            .finalize(&stranger)
            .unwrap();
        for event in [&follows, &wanted, &unwanted] {
            publisher.send_event(event).await.unwrap();
        }
        let result = fetch_following_inner(owner.public_key(), vec![url])
            .await
            .unwrap();
        assert!(result.follow_list_found);
        assert_eq!(result.follow_count, 1);
        assert_eq!(result.posts.len(), 1);
        assert_eq!(result.posts[0].id, wanted.id.to_hex());
        publisher.shutdown().await;
    }
}
