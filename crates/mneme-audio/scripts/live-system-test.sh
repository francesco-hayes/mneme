#!/bin/sh
# Runs the ignored live system-audio tests from an app bundle of their own.
#
# Why a bundle: macOS grants the System Audio Recording permission per app,
# and reads the prompt's text from that app's NSAudioCaptureUsageDescription.
# A test binary started from a terminal has no Info.plist, so macOS holds the
# terminal app responsible instead. This script wraps the test binary in a
# minimal .app that carries the key, signs it ad hoc, and starts it through
# LaunchServices (`open`), which makes the bundle responsible for itself.
#
# Usage, from anywhere inside the repository:
#
#   crates/mneme-audio/scripts/live-system-test.sh [libtest arguments]
#
#   live-system-test.sh live_system_audio_records_to_wav
#   MNEME_LIVE_SECONDS=20 live-system-test.sh live_system_audio_follows_device_changes
#   live-system-test.sh --list          # only lists the tests; captures nothing
#   MNEME_LIVE_TEST_TARGET=lib live-system-test.sh live_tap_session
#
# The bundle, its log and the WAV files go to a folder under $TMPDIR, not
# under target/. An app started this way has no access to the Documents,
# Desktop or Downloads folders unless the user grants it, and a repository
# often lives in one of them.
#
# The permission belongs to the bundle's code signature. Rebuilding the tests
# changes the ad hoc signature, so macOS may ask again; to start from a clean
# slate:  tccutil reset AudioCapture app.mneme.audio-live-test
set -eu

BUNDLE_ID="app.mneme.audio-live-test"
TARGET="${MNEME_LIVE_TEST_TARGET:-live_system}"

cd "$(dirname "$0")/.."

if [ "$TARGET" = "lib" ]; then
    SELECT="--lib"
    PATTERN="mneme_audio-"
else
    SELECT="--test $TARGET"
    PATTERN="$TARGET-"
fi

# Build, and ask cargo where it put the test binary.
# shellcheck disable=SC2086
BINARY=$(cargo test -p mneme-audio $SELECT --no-run --message-format=json \
    | sed -n "s/.*\"executable\":\"\([^\"]*\/$PATTERN[^\"\/]*\)\".*/\1/p" \
    | tail -n 1)
if [ -z "$BINARY" ] || [ ! -x "$BINARY" ]; then
    echo "could not find the built test binary for '$TARGET'" >&2
    exit 1
fi

WORK="${TMPDIR:-/tmp}"
WORK="${WORK%/}/mneme-audio-live-test"
APP="$WORK/MnemeAudioLiveTest.app"
LOG="$WORK/output.log"
EXECUTABLE="$APP/Contents/MacOS/mneme-audio-live-test"

mkdir -p "$WORK"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$BINARY" "$EXECUTABLE"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleName</key>
    <string>Mneme audio live test</string>
    <key>CFBundleExecutable</key>
    <string>mneme-audio-live-test</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>14.4</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSAudioCaptureUsageDescription</key>
    <string>Mneme's audio tests record what this Mac is playing to check system audio capture.</string>
</dict>
</plist>
PLIST
codesign --force --sign - "$APP" >/dev/null 2>&1 || {
    echo "codesign failed for $APP" >&2
    exit 1
}

: > "$LOG"
echo "running $APP"
echo "recordings and the log go to $WORK"

# Show the tests' output as it comes: they say when to play audio.
tail -f "$LOG" &
TAIL=$!
trap 'kill "$TAIL" 2>/dev/null || true' EXIT

# -W waits for the app to exit, -n starts a fresh instance, -g keeps it in
# the background. Test settings in the environment are passed on.
set -- --ignored --nocapture --test-threads=1 "$@"
ENV_ARGS="--env MNEME_LIVE_OUT_DIR=$WORK"
for NAME in MNEME_LIVE_SECONDS MNEME_SKIP_TAP_ACCESS_CHECK; do
    eval "VALUE=\${$NAME-}"
    if [ -n "$VALUE" ]; then
        ENV_ARGS="$ENV_ARGS --env $NAME=$VALUE"
    fi
done
# shellcheck disable=SC2086
open -W -n -g $ENV_ARGS --stdout "$LOG" --stderr "$LOG" "$APP" --args "$@" || true
# In case `open` could not wait for it.
while pgrep -f "$EXECUTABLE" >/dev/null 2>&1; do
    sleep 1
done
sleep 1
kill "$TAIL" 2>/dev/null || true
wait "$TAIL" 2>/dev/null || true
trap - EXIT

# `open` does not pass on the app's exit status; libtest's summary does.
if grep -q "test result: ok" "$LOG"; then
    exit 0
fi
if grep -q "benchmarks$" "$LOG" && ! grep -q "test result:" "$LOG"; then
    exit 0 # --list
fi
echo "the tests did not pass; see $LOG" >&2
exit 1
