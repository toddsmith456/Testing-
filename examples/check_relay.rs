//! Opt-in manual smoke test. Usage: cargo run --example check_relay -- <public-key> <wss-relay>
//! Contacts the specified relay and discloses the selected public key and filters to it.
use nostur_rust::core::{fetch_following, parse_public_key, parse_relays};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("Expected a public key and a wss relay URL. Never pass a private key.".into());
    }
    let key = parse_public_key(&args[0])?;
    let relays = parse_relays(&args[1])?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = rt.block_on(fetch_following(key, relays))?;
    println!(
        "follow_list_found={} follow_count={} posts={} truncated_follows={}",
        result.follow_list_found,
        result.follow_count,
        result.posts.len(),
        result.truncated_follows
    );
    Ok(())
}
