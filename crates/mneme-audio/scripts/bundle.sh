# Shared by live-system-test.sh and record.sh; sourced, not run.
#
# Wraps a command-line binary in a minimal .app and runs it there.
#
# Why a bundle: macOS grants the Microphone and System Audio Recording
# permissions per app, and reads the text of each prompt from that app's
# Info.plist (NSMicrophoneUsageDescription, NSAudioCaptureUsageDescription).
# A binary started from a terminal has no Info.plist, so macOS holds the
# terminal app responsible instead. A bundle started through LaunchServices
# (`open`) is responsible for itself.
#
# The bundle and its log go to a folder under $TMPDIR, not under target/: an
# app started this way has no access to the Documents, Desktop or Downloads
# folders unless the user grants it, and a repository often lives in one of
# them.
#
# The permissions belong to the bundle's code signature. Rebuilding the
# binary changes the ad hoc signature, so macOS may ask again.

# run_in_bundle [arguments for the binary]
#
# Set before calling:
#   BUNDLE_BINARY       the built executable to wrap
#   BUNDLE_APP          name of the bundle, without ".app"
#   BUNDLE_EXECUTABLE   name of the executable inside the bundle
#   BUNDLE_ID           bundle identifier (what `tccutil reset` takes)
#   BUNDLE_NAME         display name; the permission prompts show it
#   BUNDLE_PURPOSE      why it records, for the prompts: "... to <purpose>."
#   BUNDLE_WORK         folder for the bundle and its log
#   BUNDLE_ENV          optional: "NAME=VALUE NAME=VALUE" passed to the app
#                       (values without spaces)
#   BUNDLE_FORWARD_INT  optional: 1 passes Ctrl-C on to the app as SIGINT
#                       instead of leaving it running
#
# Shows the app's output as it comes and returns when the app has exited.
# Afterwards BUNDLE_LOG is the file holding that output. `open` does not pass
# on the app's exit status; the caller judges the run by the log.
run_in_bundle() {
    BUNDLE_PATH="$BUNDLE_WORK/$BUNDLE_APP.app"
    BUNDLE_LOG="$BUNDLE_WORK/output.log"
    bundle_exe="$BUNDLE_PATH/Contents/MacOS/$BUNDLE_EXECUTABLE"

    mkdir -p "$BUNDLE_WORK"
    rm -rf "$BUNDLE_PATH"
    mkdir -p "$BUNDLE_PATH/Contents/MacOS"
    cp "$BUNDLE_BINARY" "$bundle_exe"
    cat > "$BUNDLE_PATH/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleName</key>
    <string>$BUNDLE_NAME</string>
    <key>CFBundleExecutable</key>
    <string>$BUNDLE_EXECUTABLE</string>
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
    <string>$BUNDLE_NAME records what this Mac is playing to $BUNDLE_PURPOSE.</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>$BUNDLE_NAME records the microphone to $BUNDLE_PURPOSE.</string>
</dict>
</plist>
PLIST
    codesign --force --sign - "$BUNDLE_PATH" >/dev/null 2>&1 || {
        echo "codesign failed for $BUNDLE_PATH" >&2
        return 1
    }

    : > "$BUNDLE_LOG"
    echo "running $BUNDLE_PATH"
    echo "its output is also in $BUNDLE_LOG"

    # Show the app's output as it comes. (A background job of a script
    # ignores Ctrl-C, so this keeps showing output while the app shuts down.)
    tail -f "$BUNDLE_LOG" &
    bundle_tail=$!
    trap 'kill "$bundle_tail" 2>/dev/null || true' EXIT
    if [ "${BUNDLE_FORWARD_INT:-0}" = "1" ]; then
        # Ctrl-C reaches this script, not the app: `open` started it outside
        # the terminal's process group. Pass it on and keep waiting.
        trap 'pkill -INT -f "$bundle_exe" 2>/dev/null || true' INT TERM
    fi

    bundle_env=""
    for bundle_pair in ${BUNDLE_ENV:-}; do
        bundle_env="$bundle_env --env $bundle_pair"
    done
    # -W waits for the app to exit, -n starts a fresh instance, -g keeps it
    # in the background. In the background here so that a trapped signal is
    # handled at once rather than when `open` returns.
    # shellcheck disable=SC2086
    open -W -n -g $bundle_env --stdout "$BUNDLE_LOG" --stderr "$BUNDLE_LOG" \
        "$BUNDLE_PATH" --args "$@" &
    wait $! 2>/dev/null || true
    # `open` may be gone before the app is (after a signal, or if it could
    # not wait for it).
    while pgrep -f "$bundle_exe" >/dev/null 2>&1; do
        sleep 1 || true
    done
    sleep 1 || true
    kill "$bundle_tail" 2>/dev/null || true
    wait "$bundle_tail" 2>/dev/null || true
    trap - EXIT INT TERM
}
