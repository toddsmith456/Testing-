//! Explicit NIP-46 signing and signed-event publishing. No local account keys are stored.
use crate::core::{bounded_client, parse_relays, relay_limits};
use nostr_connect::prelude::*;
use nostr_sdk::prelude::*;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PublishError {
    #[error("Enter a valid bunker:// URI with one or more secure relay URLs")]
    InvalidBunker,
    #[error("A public post must contain 1–16,384 bytes of text")]
    InvalidContent,
    #[error("The reply target is invalid or its thread root is ambiguous")]
    InvalidReply,
    #[error("The signer did not return the selected account")]
    WrongAccount,
    #[error("The signer returned a changed or invalid event")]
    InvalidSignature,
    #[error("Signer connection or approval failed or timed out")]
    Signer,
    #[error("A signed event is invalid or belongs to a different account")]
    InvalidQueuedEvent,
    #[error("No selected relay could be connected")]
    NoRelay,
    #[error("Invalid relay selection")]
    InvalidRelays,
}

/// Expose only the host/paths; the URI itself can contain an authorization secret.
pub fn signer_relays(uri: &str) -> Result<Vec<String>, PublishError> {
    let uri = NostrConnectUri::parse(uri.trim()).map_err(|_| PublishError::InvalidBunker)?;
    if !matches!(uri, NostrConnectUri::Bunker { .. }) {
        return Err(PublishError::InvalidBunker);
    }
    let relays: Vec<String> = uri.relays().iter().map(ToString::to_string).collect();
    parse_relays(&relays.join("\n")).map_err(|_| PublishError::InvalidBunker)
}

pub fn build_note(account: PublicKey, content: &str) -> Result<UnsignedEvent, PublishError> {
    if content.trim().is_empty() || content.len() > 16_384 {
        return Err(PublishError::InvalidContent);
    }
    Ok(EventBuilder::new(Kind::TextNote, content).finalize_unsigned(account))
}

/// Construct a NIP-10 reply from a previously verified parent, without following relay hints.
pub fn build_reply(
    account: PublicKey,
    content: &str,
    parent_json: &str,
) -> Result<UnsignedEvent, PublishError> {
    use std::collections::BTreeSet;
    if content.trim().is_empty() || content.len() > 16_384 {
        return Err(PublishError::InvalidContent);
    }
    if parent_json.len() > 160_000 {
        return Err(PublishError::InvalidReply);
    }
    let parent = Event::from_json(parent_json).map_err(|_| PublishError::InvalidReply)?;
    if parent.kind != Kind::TextNote || parent.verify().is_err() {
        return Err(PublishError::InvalidReply);
    }
    let mut marked_root: Option<(String, Option<String>)> = None;
    let mut unmarked_root: Option<(String, Option<String>)> = None;
    let mut marked_reply = false;
    let mut participants = BTreeSet::new();
    participants.insert(parent.pubkey);
    for tag in parent.tags.iter() {
        let p = tag.as_slice();
        if p.first().map(String::as_str) == Some("p") {
            if let Some(key) = p.get(1).and_then(|x| PublicKey::parse(x).ok()) {
                participants.insert(key);
            }
        }
        if p.first().map(String::as_str) != Some("e") {
            continue;
        }
        let Some(id) = p.get(1).and_then(|x| EventId::parse(x).ok()) else {
            continue;
        };
        let author = p
            .get(4)
            .and_then(|x| PublicKey::parse(x).ok())
            .map(|x| x.to_hex());
        match p.get(3).map(String::as_str).unwrap_or("") {
            "root" => {
                if marked_root.is_none() {
                    marked_root = Some((id.to_hex(), author));
                }
            }
            "reply" => marked_reply = true,
            "" => {
                if unmarked_root.is_none() {
                    unmarked_root = Some((id.to_hex(), author));
                }
            }
            _ => {}
        }
    }
    if participants.len() > 200 {
        return Err(PublishError::InvalidReply);
    }
    let root = marked_root.or(unmarked_root);
    if marked_reply && root.is_none() {
        return Err(PublishError::InvalidReply);
    }
    let (root_id, root_author) =
        root.unwrap_or_else(|| (parent.id.to_hex(), Some(parent.pubkey.to_hex())));
    let mut root_tag = vec!["e".to_string(), root_id.clone(), "".into(), "root".into()];
    if let Some(author) = root_author {
        root_tag.push(author);
    }
    let mut tags = vec![Tag::parse(root_tag).map_err(|_| PublishError::InvalidReply)?];
    if root_id != parent.id.to_hex() {
        tags.push(
            Tag::parse(vec![
                "e".to_string(),
                parent.id.to_hex(),
                "".into(),
                "reply".into(),
                parent.pubkey.to_hex(),
            ])
            .map_err(|_| PublishError::InvalidReply)?,
        );
    }
    for key in participants {
        tags.push(Tag::public_key(key));
    }
    Ok(EventBuilder::new(Kind::TextNote, content)
        .tags(tags)
        .finalize_unsigned(account))
}

pub fn prepare_note(
    account: PublicKey,
    content: &str,
    reply: Option<&str>,
) -> Result<UnsignedEvent, PublishError> {
    match reply {
        Some(parent) => build_reply(account, content, parent),
        None => build_note(account, content),
    }
}

pub fn check_signer_response(expected: &UnsignedEvent, signed: &Event) -> Result<(), PublishError> {
    if signed.pubkey != expected.pubkey
        || signed.id != expected.compute_id()
        || signed.verify().is_err()
    {
        return Err(PublishError::InvalidSignature);
    }
    Ok(())
}

/// One-shot connection: the URI, connection secret, and generated client key are not saved.
pub async fn sign_remote(
    uri: &str,
    account: PublicKey,
    content: &str,
) -> Result<String, PublishError> {
    signer_relays(uri)?;
    let uri = NostrConnectUri::parse(uri.trim()).map_err(|_| PublishError::InvalidBunker)?;
    sign_remote_inner(uri, account, content).await
}

async fn sign_remote_inner(
    uri: NostrConnectUri,
    account: PublicKey,
    content: &str,
) -> Result<String, PublishError> {
    let expected = build_note(account, content)?;
    sign_expected_inner(uri, expected).await
}

pub async fn sign_expected(uri: &str, expected: UnsignedEvent) -> Result<String, PublishError> {
    signer_relays(uri)?;
    if expected.kind != Kind::TextNote
        || expected.content.trim().is_empty()
        || expected.content.len() > 16_384
    {
        return Err(PublishError::InvalidContent);
    }
    let uri = NostrConnectUri::parse(uri.trim()).map_err(|_| PublishError::InvalidBunker)?;
    sign_expected_inner(uri, expected).await
}

async fn sign_expected_inner(
    uri: NostrConnectUri,
    expected: UnsignedEvent,
) -> Result<String, PublishError> {
    let account = expected.pubkey;
    let signer = NostrConnect::new(
        uri,
        Keys::generate(),
        Duration::from_secs(45),
        Some(RelayOptions::default().limits(relay_limits())),
    )
    .map_err(|_| PublishError::Signer)?;
    let actual = signer
        .get_public_key_async()
        .await
        .map_err(|_| PublishError::Signer)?;
    if actual != account {
        return Err(PublishError::WrongAccount);
    }
    let signed = signer
        .sign_event_async(expected.clone())
        .await
        .map_err(|_| PublishError::Signer)?;
    check_signer_response(&expected, &signed)?;
    Ok(signed.as_json())
}

pub fn checked_event(account: PublicKey, event_json: &str) -> Result<Event, PublishError> {
    if event_json.len() > 160_000 {
        return Err(PublishError::InvalidQueuedEvent);
    }
    let event = Event::from_json(event_json).map_err(|_| PublishError::InvalidQueuedEvent)?;
    if event.pubkey != account
        || event.kind != Kind::TextNote
        || event.content.len() > 16_384
        || event.verify().is_err()
    {
        return Err(PublishError::InvalidQueuedEvent);
    }
    Ok(event)
}

#[derive(Debug, Clone)]
pub struct PublishReport {
    pub accepted: Vec<String>,
    pub not_confirmed: Vec<String>,
}

pub async fn publish_queued(
    account: PublicKey,
    event_json: &str,
    relays: Vec<String>,
) -> Result<PublishReport, PublishError> {
    let relays = parse_relays(&relays.join("\n")).map_err(|_| PublishError::InvalidRelays)?;
    publish_inner(account, event_json, relays).await
}

async fn publish_inner(
    account: PublicKey,
    event_json: &str,
    relays: Vec<String>,
) -> Result<PublishReport, PublishError> {
    let event = checked_event(account, event_json)?;
    let client = bounded_client();
    for url in &relays {
        client
            .add_relay(url.as_str())
            .await
            .map_err(|_| PublishError::InvalidRelays)?;
    }
    client.connect().and_wait(Duration::from_secs(8)).await;
    if !client
        .relays()
        .await
        .values()
        .any(|r| r.status().is_connected())
    {
        client.disconnect().await;
        return Err(PublishError::NoRelay);
    }
    let mut report = PublishReport {
        accepted: vec![],
        not_confirmed: vec![],
    };
    // One request per relay. Errors/timeouts are unknown, not proof of non-delivery.
    for relay in relays {
        match tokio::time::timeout(
            Duration::from_secs(10),
            client.send_event(&event).to([relay.as_str()]),
        )
        .await
        {
            Ok(Ok(output))
                if output
                    .success
                    .values()
                    .any(|v| matches!(v, EventSendStatus::Ack(_))) =>
            {
                report.accepted.push(relay)
            }
            _ => report.not_confirmed.push(relay),
        }
    }
    client.disconnect().await;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_connect::signer::{
        NostrConnectKeys, NostrConnectRemoteSigner, NostrConnectSignerActions,
    };
    struct AllowTest;
    impl NostrConnectSignerActions for AllowTest {
        fn approve(&self, _: &PublicKey, _: &NostrConnectRequest) -> bool {
            true
        }
    }
    #[test]
    fn refuses_wrong_or_mutated_signer_responses() {
        let alice = Keys::generate();
        let bob = Keys::generate();
        let expected = build_note(alice.public_key(), "hello").unwrap();
        let signed = expected.clone().finalize(&alice).unwrap();
        assert!(check_signer_response(&expected, &signed).is_ok());
        let changed = build_note(alice.public_key(), "different")
            .unwrap()
            .finalize(&alice)
            .unwrap();
        assert!(check_signer_response(&expected, &changed).is_err());
        let wrong = build_note(bob.public_key(), "hello")
            .unwrap()
            .finalize(&bob)
            .unwrap();
        assert!(check_signer_response(&expected, &wrong).is_err());
        let mut tampered = signed;
        tampered.content = "tampered".into();
        assert!(check_signer_response(&expected, &tampered).is_err());
    }
    #[test]
    fn rejects_insecure_signer_endpoints_and_empty_notes() {
        assert!(signer_relays("bunker://bad?relay=ws%3A%2F%2Flocalhost").is_err());
        assert!(build_note(Keys::generate().public_key(), "   ").is_err());
    }
    #[test]
    fn reply_tags_preserve_root_parent_and_participants_without_relay_hints() {
        let root_author = Keys::generate();
        let parent_author = Keys::generate();
        let sender = Keys::generate();
        let extra = Keys::generate();
        let root = EventBuilder::new(Kind::TextNote, "root")
            .finalize(&root_author)
            .unwrap();
        let root_tag = Tag::parse(vec![
            "e",
            &root.id.to_hex(),
            "wss://relay.example",
            "root",
            &root.pubkey.to_hex(),
        ])
        .unwrap();
        let parent = EventBuilder::new(Kind::TextNote, "parent")
            .tags([root_tag, Tag::public_key(extra.public_key())])
            .finalize(&parent_author)
            .unwrap();
        let reply = build_reply(sender.public_key(), "reply", &parent.as_json()).unwrap();
        let values: Vec<_> = reply.tags.iter().map(|t| t.as_slice().to_vec()).collect();
        assert_eq!(values[0][1], root.id.to_hex());
        assert_eq!(values[0][2], "");
        assert_eq!(values[0][3], "root");
        assert_eq!(values[1][1], parent.id.to_hex());
        assert_eq!(values[1][3], "reply");
        assert!(
            values
                .iter()
                .any(|v| v.first().map(String::as_str) == Some("p")
                    && v.get(1) == Some(&parent_author.public_key().to_hex()))
        );
        assert!(
            values
                .iter()
                .any(|v| v.first().map(String::as_str) == Some("p")
                    && v.get(1) == Some(&extra.public_key().to_hex()))
        );
        let direct = build_reply(sender.public_key(), "direct", &root.as_json()).unwrap();
        let events: Vec<_> = direct
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(String::as_str) == Some("e"))
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].as_slice()[3], "root");
        let mut tampered = parent.as_json();
        tampered = tampered.replace("parent", "tampered");
        assert!(build_reply(sender.public_key(), "reply", &tampered).is_err());
    }
    #[test]
    fn legacy_root_and_ambiguous_threads_are_handled_safely() {
        let keys = Keys::generate();
        let sender = Keys::generate();
        let root = EventId::from_byte_array([7; 32]);
        let legacy = EventBuilder::new(Kind::TextNote, "legacy")
            .tags([Tag::parse(vec!["e", &root.to_hex()]).unwrap()])
            .finalize(&keys)
            .unwrap();
        let reply = build_reply(sender.public_key(), "x", &legacy.as_json()).unwrap();
        assert_eq!(
            reply.tags.iter().next().unwrap().as_slice()[1],
            root.to_hex()
        );
        let ambiguous = EventBuilder::new(Kind::TextNote, "ambiguous")
            .tags([Tag::parse(vec!["e", &root.to_hex(), "", "reply"]).unwrap()])
            .finalize(&keys)
            .unwrap();
        assert!(build_reply(sender.public_key(), "x", &ambiguous.as_json()).is_err());
    }
    #[tokio::test]
    async fn mock_remote_signer_and_relay_end_to_end() {
        let relay = MockRelay::run().await.unwrap();
        let url = relay.url().await;
        let owner = Keys::generate();
        let other = Keys::generate();
        let bunker = NostrConnectRemoteSigner::new(
            NostrConnectKeys::new(Keys::generate(), owner.clone()),
            [url.clone()],
            Some("test-secret".into()),
            None,
        )
        .unwrap();
        let uri = bunker.bunker_uri();
        let task = tokio::spawn(async move {
            let _ = bunker.serve(AllowTest).await;
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mismatch = sign_remote_inner(uri.clone(), other.public_key(), "should not sign").await;
        assert!(matches!(mismatch, Err(PublishError::WrongAccount)));
        let json = sign_remote_inner(uri.clone(), owner.public_key(), "hello from a test")
            .await
            .unwrap();
        assert_eq!(
            checked_event(owner.public_key(), &json).unwrap().content,
            "hello from a test"
        );
        assert!(checked_event(other.public_key(), &json).is_err());
        let report = publish_inner(owner.public_key(), &json, vec![url.to_string()])
            .await
            .unwrap();
        assert_eq!(report.accepted.len(), 1);
        assert!(report.not_confirmed.is_empty());
        task.abort();
    }
}
