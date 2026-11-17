# Parity and release status

Reference: the user's `Nostur-Blueprint/01-Product-and-Design-Blueprint.md`, based on official imagery and pinned iOS source commit `e1f4ee79b62488c3889f718c51e8808ed66fa3ad`. The reference was not recorded from a live iOS device. Full clone/parity cannot be honestly claimed until the atlas and real-device interop tests described there are complete.

| Area | Status | Next work |
|---|---|---|
| Android app | Rust NativeActivity build target; packaging in development | Real device boot, keyboard, accessibility, lifecycle, font scale, low memory and network validation |
| Identity | One read-only public key | Multiple accounts and account-safe switching; external NIP-55 and NIP-46 signing with verified output |
| Relay | Manually selected WSS, finite SDK fetch | Bounded streaming, per-relay status, EOSE, timeouts, auth, retry, user-controlled outbox/discovery |
| Following | Read-only kind-1, follow-list query, local cache | Profiles, avatars/media (opt-in), thread view, reliable paging, privacy-aware errors, follows and WoT |
| Bookmarks | Local signed-post bookmarks | Colors, private notes, complete search, safe iOS/Android migration |
| Search | Currently cached feed only | User-opt-in relay searches, contacts, hashtags, QR and event references |
| Compose | Not implemented | NIP-55/NIP-46 signer, signature and payload verification, durable per-relay publish queue |
| DMs | Not implemented | NIP-17/NIP-44/59 interop, explicit legacy policy, secure storage and independent review |
| Media / notifications / zaps / polls | Not implemented | User-approved media hosts, permission & background policies, wallet safety and interoperability |
| UI fidelity | Development UI only | Reference screenshot atlas, themes, navigation, typography, accessibility and tablet/foldable |
| Security / privacy | No intentional analytics, selected relays, no automatic remote images | Audit dependency/network/manifest, resource bounds, keystore and backups; third-party security review |
| iOS private-state sync | Not implemented | Export or shared encrypted protocol; requires compatible iOS work |

## Gates

Do not label a button or screen 'complete' until source-based behavior, error/offline states, privacy effects, persistence, accessibility and real-device testing are documented. For publishing, messages and payments, require signer interoperability and independent security review. Protect private keys and wallet data, avoid automatic external contact, and never imply a relay success guarantees delivery.
