# Parity and release status

Reference: the user's `Nostur-Blueprint/01-Product-and-Design-Blueprint.md`, based on official imagery and pinned iOS source commit `e1f4ee79b62488c3889f718c51e8808ed66fa3ad`. The reference was not recorded from a live iOS device. Full clone/parity cannot be honestly claimed until the atlas and real-device interop tests described there are complete.

| Area | Status | Next work |
|---|---|---|
| Android app | Rust NativeActivity build target; packaging in development | Real device boot, keyboard, accessibility, lifecycle, font scale, low memory and network validation |
| Identity | Multiple locally saved public accounts, offline switching and experimental NIP-46 bunker signing with verified output | Account management UX, external NIP-55 and real-signer interoperability |
| Relay | Manually selected WSS, bounded finite SDK fetch | Streaming, per-relay read status, EOSE, timeouts, auth, retry, user-controlled outbox/discovery |
| Following | Kind-1, follow-list query, local cache and signed profile names | Full profiles, avatars/media (opt-in), thread view, reliable paging, privacy-aware errors, follows and WoT |
| Bookmarks | Local signed-post bookmarks | Colors, private notes, complete search, safe iOS/Android migration |
| Search | Currently cached feed only | User-opt-in relay searches, contacts, hashtags, QR and event references |
| Compose | Experimental NIP-46 signer, public NIP-10 replies, in-memory account drafts, validated signed events, durable outbox and per-relay results | Thread fetching, durable drafts, NIP-55, real-device and signer tests, safety review |
| Local mutes | Per-account local hide/unhide in Following and Search; bookmarks and profiles remain visible | Synced protocol mute lists, broader moderation, safety review |
| DMs | Not implemented | NIP-17/NIP-44/59 interop, explicit legacy policy, secure storage and independent review |
| Media / notifications / zaps / polls | Not implemented | User-approved media hosts, permission & background policies, wallet safety and interoperability |
| UI fidelity | Development UI only | Reference screenshot atlas, themes, navigation, typography, accessibility and tablet/foldable |
| Security / privacy | No intentional analytics, selected relays, no automatic remote images | Audit dependency/network/manifest, resource bounds, keystore and backups; third-party security review |
| iOS private-state sync | Not implemented | Export or shared encrypted protocol; requires compatible iOS work |

## Gates

Do not label a button or screen 'complete' until source-based behavior, error/offline states, privacy effects, persistence, accessibility and real-device testing are documented. For publishing, messages and payments, require signer interoperability and independent security review. Protect private keys and wallet data, avoid automatic external contact, and never imply a relay success guarantees delivery.

## Latest verification and release blockers

- 21 automated Rust tests pass, including account isolation, migrations, reply tags, an in-process mock relay and mock NIP-46 signer. They do not replace real signer/relay interoperability or Android device testing.
- ARM64 development APK cross-compiles and its signature and 16 KB native library alignment verify. It is debug-signed and is **not** a production release.
- A user reported that the v0.1.0-dev.2 APK would not install on Android 17 ARM64. That native-only APK did not declare `android:hasCode="false"`; a subsequent manifest fix is built and verified structurally, but still requires device installation/launch testing.
- An attempted Android emulator could not complete startup in this environment: there is no hardware virtualization and the available memory is insufficient. No Android UI, soft keyboard, installation or lifecycle behavior was verified.
- Remaining release blockers include Android real-device tests (especially text input and TalkBack in NativeActivity), real remote signer and relay tests, resilient network/error handling, security and dependency review, privacy review, user-controlled production signing, and the substantial missing parity features above. No release tag should be advertised as a complete clone before these gates pass.

Current development build: `0.1.0-dev`; its public note composer is deliberately separated into sign → persist → explicit publish. The signer URI and ephemeral NIP-46 client key are memory-only. Never confuse this feature subset or a debug APK with the complete product or a stable release.
