#!/usr/bin/env bash
# Build a developer-only ARM64 NativeActivity APK. No Gradle, Java or Kotlin app code.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
: "${ANDROID_HOME:?Set ANDROID_HOME to your Android SDK directory}"
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME to your Android NDK directory}"
: "${JAVA_HOME:?Set JAVA_HOME to a JDK directory}"
BT="$ANDROID_HOME/build-tools/35.0.0"
TOOL="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TOOL/aarch64-linux-android24-clang"
export CC_aarch64_linux_android="$TOOL/aarch64-linux-android24-clang"
export AR_aarch64_linux_android="$TOOL/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS='-C link-arg=-Wl,-z,max-page-size=16384'
cd "$ROOT"
cargo build --locked --lib --target aarch64-linux-android
OUT="$ROOT/target/android-apk"; rm -rf "$OUT"; mkdir -p "$OUT/lib/arm64-v8a" "$HOME/.android"
cp "$ROOT/target/aarch64-linux-android/debug/libnostur_rust.so" "$OUT/lib/arm64-v8a/"
"$BT/aapt2" link -o "$OUT/base.apk" -I "$ANDROID_HOME/platforms/android-35/android.jar" --manifest "$ROOT/android/AndroidManifest.xml" --min-sdk-version 24 --target-sdk-version 35
(cd "$OUT" && zip -q -0 base.apk lib/arm64-v8a/libnostur_rust.so)
"$BT/zipalign" -P 16 -f 4 "$OUT/base.apk" "$OUT/aligned.apk"
KEYSTORE="$HOME/.android/nostur-rust-debug.keystore"
if [[ ! -e "$KEYSTORE" ]]; then "$JAVA_HOME/bin/keytool" -genkeypair -keystore "$KEYSTORE" -storepass android -keypass android -alias androiddebugkey -dname 'CN=Android Debug,O=Android,C=US' -keyalg RSA -keysize 2048 -validity 10000 >/dev/null 2>&1; fi
"$BT/apksigner" sign --ks "$KEYSTORE" --ks-pass pass:android --key-pass pass:android --out "$OUT/nostur-rust-debug.apk" "$OUT/aligned.apk"
"$BT/apksigner" verify --verbose "$OUT/nostur-rust-debug.apk"
echo "APK: $OUT/nostur-rust-debug.apk"
