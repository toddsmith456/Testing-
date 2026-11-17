# Nostur-inspired Android client — Rust development build

This repository is a **work in progress**, not a completed Nostur clone. It is an unofficial independent project, not endorsed by Nostur. The blueprint's intended destination is broad product and visual parity, with stricter privacy defaults. See [parity status](docs/PARITY.md) before using or distributing it.

## What exists now

- A Rust-rendered Following view using egui/eframe, with an Android `NativeActivity` entry point and desktop development target.
- Explicit user-selected `wss://` relays; no app connection at startup. Fetches a signed NIP-02 follow list and signed kind-1 notes; results can be partial.
- Local SQLite storage for public account ID, chosen relay URLs, cached signed public posts and per-account local bookmarks. Cached events and profile metadata are signature-checked again on read. Local search scans the current cached feed; it does not query a search service.
- Relay selection and public key remain on device; public posting uses an experimental NIP-46 remote signer and a durable signed-event outbox; there is no private-key entry. **Do not enter your private key**, even as hexadecimal: a private key's raw hex can be indistinguishable from a public key by syntax.
- It does not intentionally include ads, analytics, crash collection, Google push or external media loading. **This is not an independent security, dependency, or network audit.** Relay operators see the connecting IP address, selected public key and subscription filters. Operating system, network provider and compromised-device threats are outside its control.

## Important limitations

No iOS-reference runtime comparison, Android device/emulator testing, public-relay interoperability testing, accessibility audit, independent security review, production signing, or dependency audit has been completed. A local mock-relay test exercises the signed follow-list and note retrieval flow. A public-relay attempt could not connect from this environment. The Rust interface is not Android's standard view toolkit and Android keyboard/TalkBack compatibility are unverified. Relay results may be incomplete; the SDK enforces application-level event/message and fetch-count limits, but hostile-relay resource handling still requires a security review. Signed public outbox events and their relay results are stored locally; unsigned drafts and bunker URIs are not persisted. The database is not encrypted; it must not be used for private messages, secret keys, wallet credentials or signer tokens. Bookmarks here are local-only and do not sync to iOS. The bounded feed cache merges old posts and may include authors no longer followed; bookmarks persist separately. No claim of privacy against the relays you select.

The signer and publishing path have local mock tests but have not been independently audited or verified on an Android device or against a real remote signer. No NIP-55 signer, full profiles, feeds beyond Following, media, notifications, DMs, zaps, polls, Pro columns or sync. Visual/layout details do not yet reproduce the reference. See the status document for the actual roadmap. Do not use this build with sensitive accounts or rely on it as your sole source of user data.

## Build & test on Linux

Install stable Rust (the lockfile was tested with Rust 1.100), plus X11 and OpenGL build/runtime libraries for desktop. Then run:

```sh
cargo test --locked
cargo run --locked
```

For an Android developer APK you also need JDK 17, Android SDK platform 35, build-tools 35.0.0, Android NDK 27.2.12479018, Rust target `aarch64-linux-android`, `zip` and the Linux host NDK toolchain. Set `JAVA_HOME`, `ANDROID_HOME` and `ANDROID_NDK_HOME`, make Rust and Java available on PATH, then:

```sh
rustup target add aarch64-linux-android
./scripts/build-android.sh
```

The script produces `target/android-apk-arm64-v8a-debug/nostur-rust-debug.apk` for ARM64, signed with a **local debug key** under `~/.android`. This key is deliberately not checked in and is unsuitable for release. No claims of device operation are made merely because an APK can be assembled. APK updates require the same signing key.

## Legal

GPL-3.0-or-later; see LICENSE. Individual dependencies have separate licenses and must be audited for a release. Nostur's name, artwork and trademark are separate from GPL source rights; the development label is not a statement of endorsement.

## Release signing (not yet a production release)

The same packaging script supports `ANDROID_BUILD_MODE=release`. It requires a user-controlled keystore and alias supplied with `ANDROID_KEYSTORE` and `ANDROID_KEY_ALIAS`, and passwords supplied privately through `ANDROID_STORE_PASS` and `ANDROID_KEY_PASS` environment variables. Do not commit keys or passwords or send them in chat. The script refuses release mode without them. An optimized and signed build alone does not satisfy the security, device testing or feature-completeness gates in `docs/PARITY.md`.

For a local x86_64 Android emulator development build, set `ANDROID_ABI=x86_64`; the default is `arm64-v8a`. Outputs are under `target/android-apk-<abi>-<debug|release>/`.

### Experimental public posting

In **New post**, enter the note and a `bunker://` URI from a compatible remote signer. The app displays the signer's relay destinations; **Request signature** does not publish. After approval, the verified signed event is stored in the local outbox. **Publish / retry** sends that same event to the selected relays; it reports acknowledgements and unconfirmed destinations. An interrupted send may already have reached a relay, and removing an outbox entry does not delete a public event. No automatic retry is performed. The bunker URI, its connection secret, and the ephemeral signer client key are not stored in the database. Do not use the experimental signing path with important accounts until it has been audited and tested with real signers and devices.
