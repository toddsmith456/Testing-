#!/usr/bin/env bash
# Build an Android NativeActivity APK. Release builds require user-controlled signing material.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
: "${ANDROID_HOME:?Set ANDROID_HOME to your Android SDK directory}"
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME to your Android NDK directory}"
: "${JAVA_HOME:?Set JAVA_HOME to a JDK directory}"
# Android SDK tools launch java from PATH; use the requested JDK consistently.
export PATH="$JAVA_HOME/bin:$PATH"
BT="$ANDROID_HOME/build-tools/35.0.0"
TOOL="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
ABI="${ANDROID_ABI:-arm64-v8a}"
case "$ABI" in
  arm64-v8a) TARGET=aarch64-linux-android; export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TOOL/aarch64-linux-android24-clang" CC_aarch64_linux_android="$TOOL/aarch64-linux-android24-clang" AR_aarch64_linux_android="$TOOL/llvm-ar" CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS='-C link-arg=-Wl,-z,max-page-size=16384' ;;
  x86_64) TARGET=x86_64-linux-android; export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$TOOL/x86_64-linux-android24-clang" CC_x86_64_linux_android="$TOOL/x86_64-linux-android24-clang" AR_x86_64_linux_android="$TOOL/llvm-ar" CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS='-C link-arg=-Wl,-z,max-page-size=16384' ;;
  *) echo "Unsupported ANDROID_ABI: $ABI" >&2; exit 1 ;;
esac
MODE="${ANDROID_BUILD_MODE:-debug}"
case "$MODE" in debug) BUILD_FLAGS=();; release)
  : "${ANDROID_KEYSTORE:?Set ANDROID_KEYSTORE to your release keystore}"
  : "${ANDROID_KEY_ALIAS:?Set ANDROID_KEY_ALIAS to your release signing alias}"
  : "${ANDROID_STORE_PASS:?Set ANDROID_STORE_PASS securely in the environment}"
  : "${ANDROID_KEY_PASS:?Set ANDROID_KEY_PASS securely in the environment}"
  BUILD_FLAGS=(--release);; *) echo "ANDROID_BUILD_MODE must be debug or release" >&2;exit 1;; esac
cd "$ROOT"
cargo build --locked --lib --target "$TARGET" "${BUILD_FLAGS[@]}"
OUT="$ROOT/target/android-apk-$ABI-$MODE"; rm -rf "$OUT"; mkdir -p "$OUT/lib/$ABI" "$HOME/.android"
cp "$ROOT/target/$TARGET/$MODE/libnostur_rust.so" "$OUT/lib/$ABI/"
"$BT/aapt2" link -o "$OUT/base.apk" -I "$ANDROID_HOME/platforms/android-35/android.jar" --manifest "$ROOT/android/AndroidManifest.xml" --min-sdk-version 24 --target-sdk-version 35
(cd "$OUT" && zip -q -0 base.apk "lib/$ABI/libnostur_rust.so")
"$BT/zipalign" -P 16 -f 4 "$OUT/base.apk" "$OUT/aligned.apk"
if [[ "$MODE" == debug ]]; then
  KEYSTORE="$HOME/.android/nostur-rust-debug.keystore"
  if [[ ! -e "$KEYSTORE" ]]; then "$JAVA_HOME/bin/keytool" -genkeypair -keystore "$KEYSTORE" -storepass android -keypass android -alias androiddebugkey -dname 'CN=Android Debug,O=Android,C=US' -keyalg RSA -keysize 2048 -validity 10000 >/dev/null 2>&1; fi
  "$BT/apksigner" sign --ks "$KEYSTORE" --ks-pass pass:android --key-pass pass:android --out "$OUT/nostur-rust-debug.apk" "$OUT/aligned.apk"
else
  "$BT/apksigner" sign --ks "$ANDROID_KEYSTORE" --ks-key-alias "$ANDROID_KEY_ALIAS" --ks-pass env:ANDROID_STORE_PASS --key-pass env:ANDROID_KEY_PASS --out "$OUT/nostur-rust-release.apk" "$OUT/aligned.apk"
fi
"$BT/apksigner" verify --verbose "$OUT/nostur-rust-$MODE.apk"
# This APK has native code only. If it declares DEX code, Android can reject installation.
MANIFEST_XML=$("$BT/aapt" dump xmltree "$OUT/nostur-rust-$MODE.apk" AndroidManifest.xml)
if ! grep -Eq 'android:hasCode.*0x0' <<<"$MANIFEST_XML"; then
  echo "ERROR: packaged manifest does not declare android:hasCode=false" >&2; exit 1
fi
APK_ENTRIES=$(unzip -Z1 "$OUT/nostur-rust-$MODE.apk")
if grep -Eq '(^|/)classes([0-9]+)?\.dex$' <<<"$APK_ENTRIES"; then
  echo "ERROR: APK contains DEX but manifest declares no DEX code" >&2; exit 1
fi
echo "APK: $OUT/nostur-rust-$MODE.apk"
