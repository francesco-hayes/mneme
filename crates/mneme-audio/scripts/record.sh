#!/bin/sh
# Runs mneme-record from an app bundle of its own, so that macOS asks for
# (and remembers) the Microphone and System Audio Recording permissions for
# "Mneme record" and not for the terminal. scripts/bundle.sh explains why
# and does the wrapping.
#
# Usage, from anywhere inside the repository:
#
#   crates/mneme-audio/scripts/record.sh [--out <dir>] [--seconds <n>] [--fake]
#
#   record.sh                     # until Ctrl-C
#   record.sh --seconds 60
#   record.sh --fake --seconds 3  # synthetic tones: no device, no prompts
#   record.sh --help              # mneme-record's own help, through the bundle
#
# The arguments are mneme-record's. Without --out the files go to a new
# folder under $TMPDIR/mneme-record, because the bundle has no access to the
# Documents, Desktop and Downloads folders unless macOS is told to allow it.
# Ctrl-C is passed on to the recorder, which then finishes both files.
#
# To be asked for the permissions from scratch:
#   tccutil reset Microphone app.mneme.record
#   tccutil reset AudioCapture app.mneme.record
set -eu

ORIGIN=$PWD
cd "$(dirname "$0")/.."
# shellcheck source-path=SCRIPTDIR source=bundle.sh
. scripts/bundle.sh

WORK="${TMPDIR:-/tmp}"
WORK="${WORK%/}/mneme-record"

# Pull --out out of the arguments: the app is started with / as its working
# directory, so the folder must be absolute, and it needs a default.
OUT=""
RECORDS=1
COUNT=$#
while [ "$COUNT" -gt 0 ]; do
    ARG=$1
    shift
    COUNT=$((COUNT - 1))
    case "$ARG" in
        --out)
            if [ "$COUNT" -eq 0 ]; then
                echo "--out needs a value" >&2
                exit 2
            fi
            OUT=$1
            shift
            COUNT=$((COUNT - 1))
            ;;
        --out=*)
            OUT=${ARG#--out=}
            ;;
        -h | --help)
            RECORDS=0
            set -- "$@" "$ARG"
            ;;
        *)
            set -- "$@" "$ARG"
            ;;
    esac
done
if [ "$RECORDS" -eq 1 ]; then
    case "$OUT" in
        "") OUT="$WORK/recording-$(date +%Y%m%d-%H%M%S)" ;;
        /*) ;;
        *) OUT="$ORIGIN/$OUT" ;;
    esac
    case "$OUT" in
        "$HOME/Documents"/* | "$HOME/Desktop"/* | "$HOME/Downloads"/*)
            echo "note: $OUT is in a folder macOS protects. The bundle may be" >&2
            echo "      refused there, or macOS may ask to allow it." >&2
            ;;
    esac
    set -- "$@" --out "$OUT"
fi

# Build, and ask cargo where it put the binary.
BINARY=$(cargo build -p mneme-audio --bin mneme-record --message-format=json \
    | sed -n 's/.*"executable":"\([^"]*\/mneme-record\)".*/\1/p' \
    | tail -n 1)
if [ -z "$BINARY" ] || [ ! -x "$BINARY" ]; then
    echo "could not find the built mneme-record binary" >&2
    exit 1
fi

BUNDLE_BINARY="$BINARY"
BUNDLE_APP="MnemeRecord"
BUNDLE_EXECUTABLE="mneme-record"
BUNDLE_ID="app.mneme.record"
BUNDLE_NAME="Mneme record"
BUNDLE_PURPOSE="write it to a WAV file for testing"
BUNDLE_WORK="$WORK"
BUNDLE_FORWARD_INT=1
BUNDLE_ENV=""
if [ -n "${MNEME_SKIP_TAP_ACCESS_CHECK-}" ]; then
    BUNDLE_ENV="MNEME_SKIP_TAP_ACCESS_CHECK=$MNEME_SKIP_TAP_ACCESS_CHECK"
fi

run_in_bundle "$@"

# `open` does not pass on the app's exit status. mneme-record starts a line
# with "error:" whenever it does not exit with 0.
if [ ! -s "$BUNDLE_LOG" ]; then
    echo "mneme-record printed nothing; it may not have started" >&2
    exit 1
fi
if grep -q "^error:" "$BUNDLE_LOG"; then
    exit 1
fi
exit 0
