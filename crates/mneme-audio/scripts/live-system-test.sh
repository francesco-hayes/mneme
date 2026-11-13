#!/bin/sh
# Runs the ignored live system-audio tests from an app bundle of their own,
# so that macOS attributes the System Audio Recording permission to that
# bundle and not to the terminal. scripts/bundle.sh explains why and does
# the wrapping.
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
# The bundle, its log and the WAV files go to $TMPDIR/mneme-audio-live-test.
#
# To be asked for the permission from scratch:
#   tccutil reset AudioCapture app.mneme.audio-live-test
set -eu

TARGET="${MNEME_LIVE_TEST_TARGET:-live_system}"

cd "$(dirname "$0")/.."
# shellcheck source-path=SCRIPTDIR source=bundle.sh
. scripts/bundle.sh

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
    | sed -n "s/.*\"executable\":\"\([^\"]*\/${PATTERN}[^\"\/]*\)\".*/\1/p" \
    | tail -n 1)
if [ -z "$BINARY" ] || [ ! -x "$BINARY" ]; then
    echo "could not find the built test binary for '$TARGET'" >&2
    exit 1
fi

WORK="${TMPDIR:-/tmp}"
WORK="${WORK%/}/mneme-audio-live-test"

BUNDLE_BINARY="$BINARY"
BUNDLE_APP="MnemeAudioLiveTest"
BUNDLE_EXECUTABLE="mneme-audio-live-test"
BUNDLE_ID="app.mneme.audio-live-test"
BUNDLE_NAME="Mneme audio live test"
BUNDLE_PURPOSE="check audio capture"
BUNDLE_WORK="$WORK"
# Test settings in the environment are passed on.
BUNDLE_ENV="MNEME_LIVE_OUT_DIR=$WORK"
for NAME in MNEME_LIVE_SECONDS MNEME_SKIP_TAP_ACCESS_CHECK; do
    eval "VALUE=\${$NAME-}"
    if [ -n "$VALUE" ]; then
        BUNDLE_ENV="$BUNDLE_ENV $NAME=$VALUE"
    fi
done

echo "recordings go to $WORK"
# The tests say when to play audio; their output is shown as it comes.
run_in_bundle --ignored --nocapture --test-threads=1 "$@"

# `open` does not pass on the app's exit status; libtest's summary does.
if grep -q "test result: ok" "$BUNDLE_LOG"; then
    exit 0
fi
if grep -q "benchmarks$" "$BUNDLE_LOG" && ! grep -q "test result:" "$BUNDLE_LOG"; then
    exit 0 # --list
fi
echo "the tests did not pass; see $BUNDLE_LOG" >&2
exit 1
