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
