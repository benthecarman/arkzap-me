use crate::models::arkade_zap::ArkadeZap;
use crate::models::zap::Zap;
use crate::State;
use anyhow::{anyhow, bail, Context};
use chrono::NaiveDateTime;
use log::{error, info, warn};
use nostr::event::{Event, EventBuilder, FinalizeEvent, Kind, Tag};
use nostr::key::Keys;
use nostr::types::{RelayUrl, Timestamp};
use nostr_sdk::client::Client;
use std::time::Duration;

/// How often unpublished receipts are retried when no settlement wakes the publisher.
const RETRY_INTERVAL: Duration = Duration::from_secs(30);
/// Receipts that still fail to publish this long after settlement are abandoned.
const RETRY_WINDOW_HOURS: i32 = 24;
/// Upper bound on relays contacted per receipt, since the zap sender picks them.
const MAX_RELAYS: usize = 10;
const RELAY_TIMEOUT: Duration = Duration::from_secs(10);

/// Validates a NIP-57 zap request before an invoice is created for it.
///
/// See <https://github.com/nostr-protocol/nips/blob/master/57.md#appendix-d-lnurl-server-zap-request-validation>
pub fn validate_zap_request(event: &Event, amount_msats: u64) -> anyhow::Result<()> {
    if event.kind != Kind::ZapRequest {
        bail!("zap request has kind {}", event.kind);
    }
    event
        .verify()
        .map_err(|e| anyhow!("zap request signature is invalid: {e}"))?;

    let tag_count = |kind: &str| event.tags.iter().filter(|t| t.kind() == kind).count();
    if tag_count("p") != 1 {
        bail!("zap request must have exactly one p tag");
    }
    if tag_count("e") > 1 {
        bail!("zap request must have at most one e tag");
    }
    if tag_count("a") > 1 {
        bail!("zap request must have at most one a tag");
    }
    if tag_count("P") > 1 {
        bail!("zap request must have at most one P tag");
    }
    if zap_request_relays(event).is_empty() {
        bail!("zap request has no usable relays");
    }

    if let Some(amount) = tag_value(event, "amount") {
        let requested: u64 = amount
            .parse()
            .map_err(|_| anyhow!("zap request amount is not a number"))?;
        if requested != amount_msats {
            bail!("zap request amount {requested} does not match invoice amount {amount_msats}");
        }
    }

    Ok(())
}

/// Publishes receipts for settled zaps. Wakes on `State::zap_receipts` or every `RETRY_INTERVAL`.
pub async fn run(state: State) {
    loop {
        if let Err(e) = publish_pending(&state).await {
            error!("Unable to publish zap receipts: {e:#}");
        }

        tokio::select! {
            _ = state.zap_receipts.notified() => {}
            _ = tokio::time::sleep(RETRY_INTERVAL) => {}
        }
    }
}

async fn publish_pending(state: &State) -> anyhow::Result<()> {
    let (bark, arkade) = {
        let mut conn = state.db_pool.get()?;
        (
            Zap::get_unpublished_settled(&mut conn, RETRY_WINDOW_HOURS)?,
            ArkadeZap::get_unpublished_settled(&mut conn, RETRY_WINDOW_HOURS)?,
        )
    };

    for (zap, invoice) in bark {
        let Some(settled_at) = invoice.settled_at else {
            continue;
        };
        match publish_receipt(
            &state.keys,
            &zap.request,
            &invoice.bolt11,
            &invoice.preimage,
            settled_at,
        )
        .await
        {
            Ok(Some(event_id)) => {
                let mut conn = state.db_pool.get()?;
                zap.set_event_id(&mut conn, event_id)?;
            }
            Ok(None) => {}
            Err(e) => error!(
                "Unable to publish zap receipt for invoice {}: {e:#}",
                invoice.id
            ),
        }
    }

    for (zap, invoice) in arkade {
        let Some(settled_at) = invoice.settled_at else {
            continue;
        };
        match publish_receipt(
            &state.keys,
            &zap.request,
            &invoice.bolt11,
            &invoice.preimage,
            settled_at,
        )
        .await
        {
            Ok(Some(event_id)) => {
                let mut conn = state.db_pool.get()?;
                zap.set_event_id(&mut conn, event_id)?;
            }
            Ok(None) => {}
            Err(e) => error!(
                "Unable to publish zap receipt for Arkade invoice {}: {e:#}",
                invoice.id
            ),
        }
    }

    Ok(())
}

/// Returns the receipt's event id once at least one relay accepted it.
async fn publish_receipt(
    keys: &Keys,
    request: &str,
    bolt11: &str,
    preimage: &str,
    settled_at: NaiveDateTime,
) -> anyhow::Result<Option<String>> {
    let zap_request = Event::from_json(request).context("stored zap request is invalid")?;
    let relays = zap_request_relays(&zap_request);
    if relays.is_empty() {
        bail!("zap request has no usable relays");
    }
    let receipt = build_zap_receipt(keys, &zap_request, request, bolt11, preimage, settled_at)?;

    if send_to_relays(&receipt, &relays).await? == 0 {
        return Ok(None);
    }
    Ok(Some(receipt.id.to_hex()))
}

/// Sends `event` to `relays` and returns how many accepted it.
async fn send_to_relays(event: &Event, relays: &[RelayUrl]) -> anyhow::Result<usize> {
    let client = Client::new();
    let mut added = Vec::with_capacity(relays.len());
    for relay in relays {
        match client.add_relay(relay).await {
            Ok(_) => added.push(relay),
            Err(e) => warn!("Unable to add zap receipt relay {relay}: {e}"),
        }
    }
    if added.is_empty() {
        return Ok(0);
    }

    client.connect().and_wait(RELAY_TIMEOUT).await;
    let output = client
        .send_event(event)
        .to(added)
        .ok_timeout(RELAY_TIMEOUT)
        .await;
    client.shutdown().await;
    let output = output.context("failed to send zap receipt")?;

    if output.success.is_empty() {
        warn!(
            "No relay accepted zap receipt {} (failed: {:?})",
            event.id, output.failed
        );
    } else {
        info!(
            "Published zap receipt {} to {} relay(s)",
            event.id,
            output.success.len()
        );
    }
    Ok(output.success.len())
}

/// Builds a NIP-57 zap receipt.
///
/// `request_json` is the zap request exactly as received, since that is what the invoice
/// description commits to. `created_at` is the settlement time, so retries produce the same
/// event id and relays deduplicate them.
fn build_zap_receipt(
    keys: &Keys,
    zap_request: &Event,
    request_json: &str,
    bolt11: &str,
    preimage: &str,
    settled_at: NaiveDateTime,
) -> anyhow::Result<Event> {
    let mut tags = vec![
        Tag::parse(["bolt11", bolt11])?,
        Tag::parse(["description", request_json])?,
    ];
    if !preimage.is_empty() {
        tags.push(Tag::parse(["preimage", preimage])?);
    }
    for kind in ["p", "e", "a"] {
        if let Some(tag) = zap_request.tags.iter().find(|tag| tag.kind() == kind) {
            tags.push(tag.clone());
        }
    }
    tags.push(Tag::parse(["P".to_string(), zap_request.pubkey.to_hex()])?);

    let created_at = Timestamp::from(settled_at.and_utc().timestamp().max(0) as u64);
    EventBuilder::new(Kind::ZapReceipt, "")
        .tags(tags)
        .custom_created_at(created_at)
        .finalize(keys)
        .map_err(|e| anyhow!("failed to sign zap receipt: {e}"))
}

/// Public relays from the request's `relays` tag. Local and onion relays are skipped
/// so senders cannot point the server at internal hosts.
fn zap_request_relays(zap_request: &Event) -> Vec<RelayUrl> {
    let mut relays: Vec<RelayUrl> = Vec::new();
    let Some(tag) = zap_request.tags.iter().find(|tag| tag.kind() == "relays") else {
        return relays;
    };
    for relay in tag.as_slice().iter().skip(1) {
        let Ok(relay) = RelayUrl::parse(relay) else {
            continue;
        };
        if relay.is_localhost() || relay.is_local_addr() || relay.is_onion() {
            continue;
        }
        if !relays.contains(&relay) {
            relays.push(relay);
        }
        if relays.len() == MAX_RELAYS {
            break;
        }
    }
    relays
}

fn tag_value<'a>(event: &'a Event, kind: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|tag| tag.kind() == kind)
        .and_then(|tag| tag.content())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::local_relay::MockRelay;

    const RECIPIENT: &str = "82341f882b6eabcd2ba7f1ef90aad961cf074af15b9ef44a09f9d2a8fbfbe6a2";

    fn zap_request(keys: &Keys, tags: Vec<Vec<&str>>) -> Event {
        let tags = tags.into_iter().map(|tag| Tag::parse(tag).unwrap());
        EventBuilder::new(Kind::ZapRequest, "great post")
            .tags(tags)
            .finalize(keys)
            .unwrap()
    }

    fn valid_request(keys: &Keys) -> Event {
        zap_request(
            keys,
            vec![
                vec!["p", RECIPIENT],
                vec!["amount", "21000"],
                vec!["relays", "wss://relay.example.com", "wss://nos.example.org"],
            ],
        )
    }

    #[test]
    fn accepts_valid_zap_request() {
        let keys = Keys::generate();
        validate_zap_request(&valid_request(&keys), 21_000).unwrap();
    }

    #[test]
    fn rejects_amount_mismatch() {
        let keys = Keys::generate();
        assert!(validate_zap_request(&valid_request(&keys), 22_000).is_err());
    }

    #[test]
    fn rejects_bad_signature() {
        let keys = Keys::generate();
        let json = valid_request(&keys)
            .as_json()
            .replace("great post", "edited");
        let tampered = Event::from_json(json).unwrap();
        assert!(validate_zap_request(&tampered, 21_000).is_err());
    }

    #[test]
    fn rejects_wrong_p_tag_count() {
        let keys = Keys::generate();
        let none = zap_request(&keys, vec![vec!["relays", "wss://relay.example.com"]]);
        assert!(validate_zap_request(&none, 21_000).is_err());

        let two = zap_request(
            &keys,
            vec![
                vec!["p", RECIPIENT],
                vec!["p", RECIPIENT],
                vec!["relays", "wss://relay.example.com"],
            ],
        );
        assert!(validate_zap_request(&two, 21_000).is_err());
    }

    #[test]
    fn rejects_request_without_public_relays() {
        let keys = Keys::generate();
        let missing = zap_request(&keys, vec![vec!["p", RECIPIENT]]);
        assert!(validate_zap_request(&missing, 21_000).is_err());

        let local = zap_request(
            &keys,
            vec![
                vec!["p", RECIPIENT],
                vec!["relays", "ws://127.0.0.1:3535", "ws://localhost:7777"],
            ],
        );
        assert!(validate_zap_request(&local, 21_000).is_err());
    }

    #[test]
    fn relays_are_deduplicated_and_capped() {
        let keys = Keys::generate();
        let urls: Vec<String> = (0..15)
            .map(|i| format!("wss://relay{i}.example.com"))
            .collect();
        let mut relays_tag = vec!["relays", "wss://relay0.example.com"];
        relays_tag.extend(urls.iter().map(String::as_str));
        let request = zap_request(&keys, vec![vec!["p", RECIPIENT], relays_tag]);

        let relays = zap_request_relays(&request);
        assert_eq!(relays.len(), MAX_RELAYS);
        assert_eq!(
            relays[0].as_str_without_trailing_slash(),
            "wss://relay0.example.com"
        );
        assert_eq!(
            relays[1].as_str_without_trailing_slash(),
            "wss://relay1.example.com"
        );
    }

    #[tokio::test]
    async fn sends_receipt_to_relays() {
        let relay = MockRelay::run().await.unwrap();
        let url = relay.url().await;
        let unreachable = RelayUrl::parse("ws://127.0.0.1:1").unwrap();
        let receipt = EventBuilder::new(Kind::ZapReceipt, "")
            .finalize(&Keys::generate())
            .unwrap();

        assert_eq!(
            send_to_relays(&receipt, &[url, unreachable.clone()])
                .await
                .unwrap(),
            1
        );
        assert_eq!(send_to_relays(&receipt, &[unreachable]).await.unwrap(), 0);
    }

    #[test]
    fn receipt_follows_nip57_and_is_deterministic() {
        let sender = Keys::generate();
        let server = Keys::generate();
        let request = zap_request(
            &sender,
            vec![
                vec!["p", RECIPIENT],
                vec![
                    "e",
                    "5c83da77af1dec6d7289834998ad7aafbd9e2191396d75ec3cc27f5a77226f36",
                ],
                vec!["relays", "wss://relay.example.com"],
            ],
        );
        let request_json = request.as_json();
        let settled_at = chrono::DateTime::from_timestamp(1_700_000_000, 0)
            .unwrap()
            .naive_utc();

        let receipt = build_zap_receipt(
            &server,
            &request,
            &request_json,
            "lnbc1fake",
            "00ff",
            settled_at,
        )
        .unwrap();
        let again = build_zap_receipt(
            &server,
            &request,
            &request_json,
            "lnbc1fake",
            "00ff",
            settled_at,
        )
        .unwrap();

        receipt.verify().unwrap();
        assert_eq!(receipt.kind, Kind::ZapReceipt);
        assert_eq!(receipt.pubkey, server.public_key());
        assert_eq!(receipt.created_at.as_secs(), 1_700_000_000);
        assert_eq!(receipt.id, again.id);
        assert_eq!(tag_value(&receipt, "bolt11"), Some("lnbc1fake"));
        assert_eq!(
            tag_value(&receipt, "description"),
            Some(request_json.as_str())
        );
        assert_eq!(tag_value(&receipt, "preimage"), Some("00ff"));
        assert_eq!(tag_value(&receipt, "p"), Some(RECIPIENT));
        assert_eq!(
            tag_value(&receipt, "e"),
            Some("5c83da77af1dec6d7289834998ad7aafbd9e2191396d75ec3cc27f5a77226f36")
        );
        let sender_hex = sender.public_key().to_hex();
        assert_eq!(tag_value(&receipt, "P"), Some(sender_hex.as_str()));
    }
}
