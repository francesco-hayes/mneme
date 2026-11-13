#!/usr/bin/env bash
# Builds a release of Mneme, signs it with a Developer ID certificate, has
# Apple notarize it with `notarytool`, and staples the ticket.
#
# THIS NEEDS A PAID APPLE DEVELOPER ACCOUNT. Without a "Developer ID
# Application" certificate in your keychain and notary credentials, the script
# stops before building anything. See docs/release.md, "Release".
#
# Nothing secret is read from or written to the repository. Credentials come
# from the environment or from a keychain profile:
#
#   APPLE_SIGNING_IDENTITY   required. The certificate to sign with, for
#                            example "Developer ID Application: Jane Doe (ABCDE12345)",
#                            exactly as `security find-identity -v -p codesigning`
#                            prints it.
#
#   and one of these two for the notary service:
#
#   MNEME_NOTARY_PROFILE     name of a keychain profile made once with
#                            `xcrun notarytool store-credentials`. Preferred:
#                            the secret stays in the keychain.
#
#   APPLE_API_KEY_PATH       path to an App Store Connect API key (.p8 file,
#   APPLE_API_KEY            kept outside the repository), its key ID, and the
#   APPLE_API_ISSUER         issuer ID (leave the issuer unset for an
#                            individual key).
#
# An Apple ID with an app-specific password is supported only through a
# keychain profile, because a password given to notarytool on the command line
# is visible to every process on the machine.
#
# Usage:
#   scripts/release.sh              build, sign, notarize, staple, zip
#   scripts/release.sh --dry-run    check prerequisites and print each step;
#                                   builds nothing, signs nothing, contacts nobody
#   scripts/release.sh --help
#
# Exit codes: 0 done (or dry run with everything in place), 1 a prerequisite is
# missing or a step failed, 2 bad usage.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN_CONF="$ROOT/src-tauri/tauri.conf.json"
RELEASE_CONF="$ROOT/src-tauri/tauri.release.conf.json"
MACOS_CONF="$ROOT/src-tauri/tauri.macos.conf.json"
BUNDLE_DIR="$ROOT/target/release/bundle/macos"
DRY_RUN=0

usage() {
    sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY_RUN=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

# --- Prerequisites -----------------------------------------------------------
PROBLEMS=0
problem() {
    PROBLEMS=$((PROBLEMS + 1))
    echo "  missing: $*" >&2
}

echo "Checking prerequisites..."

[ "$(uname -s)" = "Darwin" ] || problem "macOS. This script only runs on a Mac."
for tool in pnpm cargo jq codesign ditto xcrun spctl shasum; do
    command -v "$tool" >/dev/null 2>&1 || problem "$tool on PATH."
done
if command -v xcrun >/dev/null 2>&1; then
    xcrun --find notarytool >/dev/null 2>&1 || problem "notarytool (comes with the Xcode Command Line Tools, Xcode 13 or later)."
    xcrun --find stapler >/dev/null 2>&1 || problem "stapler (comes with the Xcode Command Line Tools)."
fi

IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
if [ -z "$IDENTITY" ]; then
    problem "APPLE_SIGNING_IDENTITY. Set it to your \"Developer ID Application: ...\" certificate. That certificate needs an Apple Developer account."
else
    case "$IDENTITY" in
        "Developer ID Application:"*) ;;
        *) problem "a Developer ID identity. APPLE_SIGNING_IDENTITY must start with \"Developer ID Application:\"; other certificates cannot be notarized for distribution outside the App Store." ;;
    esac
    # Read-only keychain lookup; -v lists only identities that are valid now.
    if ! security find-identity -v -p codesigning 2>/dev/null | grep -qF "\"$IDENTITY\""; then
        problem "the certificate named by APPLE_SIGNING_IDENTITY in your keychain (security find-identity -v -p codesigning does not list it)."
    fi
fi

# How to authenticate to the notary service. Only names, IDs and a file path
# ever reach a command line; the secret itself stays in the keychain or in the
# key file.
NOTARY_AUTH=()
NOTARY_HOW=""
if [ -n "${MNEME_NOTARY_PROFILE:-}" ]; then
    NOTARY_AUTH=(--keychain-profile "$MNEME_NOTARY_PROFILE")
    NOTARY_HOW="keychain profile \"$MNEME_NOTARY_PROFILE\""
elif [ -n "${APPLE_API_KEY_PATH:-}" ] || [ -n "${APPLE_API_KEY:-}" ]; then
    if [ -z "${APPLE_API_KEY_PATH:-}" ] || [ -z "${APPLE_API_KEY:-}" ]; then
        problem "both APPLE_API_KEY_PATH and APPLE_API_KEY (only one is set)."
    elif [ ! -f "$APPLE_API_KEY_PATH" ]; then
        problem "the API key file APPLE_API_KEY_PATH points to."
    else
        case "$(cd "$(dirname "$APPLE_API_KEY_PATH")" && pwd -P)/" in
            "$ROOT"/*) problem "an API key stored outside the repository. APPLE_API_KEY_PATH points inside $ROOT; move the key file out." ;;
        esac
        NOTARY_AUTH=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY")
        if [ -n "${APPLE_API_ISSUER:-}" ]; then
            NOTARY_AUTH+=(--issuer "$APPLE_API_ISSUER")
        fi
        NOTARY_HOW="App Store Connect API key $APPLE_API_KEY"
    fi
else
    problem "notary credentials. Set MNEME_NOTARY_PROFILE (a profile made with \`xcrun notarytool store-credentials\`), or APPLE_API_KEY_PATH and APPLE_API_KEY (and APPLE_API_ISSUER for a team key)."
    if [ -n "${APPLE_ID:-}" ] || [ -n "${APPLE_PASSWORD:-}" ]; then
        echo "           (APPLE_ID / APPLE_PASSWORD are set but not used here: store them once with" >&2
        echo "            \`xcrun notarytool store-credentials\` and set MNEME_NOTARY_PROFILE.)" >&2
    fi
fi

# A hardened-runtime app can only use the microphone if its entitlements say
# so. The entitlements file belongs to issue 024; this script only checks that
# the configuration the build will use names one that exists.
ENTITLEMENTS=""
if command -v jq >/dev/null 2>&1; then
    CONFS=("$MAIN_CONF")
    [ -f "$MACOS_CONF" ] && CONFS+=("$MACOS_CONF")
    CONFS+=("$RELEASE_CONF")
    ENTITLEMENTS="$(jq -r -s 'reduce .[] as $c ({}; . * $c) | .bundle.macOS.entitlements // ""' "${CONFS[@]}")"
    if [ -z "$ENTITLEMENTS" ]; then
        problem "an entitlements file in the Tauri config (bundle.macOS.entitlements). Without it a hardened-runtime build cannot record from the microphone. Issue 024 adds it."
    elif [ ! -f "$ROOT/src-tauri/$ENTITLEMENTS" ] && [ ! -f "$ENTITLEMENTS" ]; then
        problem "the entitlements file the config names ($ENTITLEMENTS)."
    fi
fi

# Tauri notarizes by itself when it sees these, which would bypass the checks
# below and, for APPLE_PASSWORD, put a password on a command line. The build
# step therefore runs without them; this script does the notarizing.
HIDE_FROM_TAURI=(-u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID -u APPLE_API_KEY
    -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD)

# Print a command, and run it unless this is a dry run.
run() {
    printf '  $'
    printf ' %q' "$@"
    printf '\n'
    if [ "$DRY_RUN" -eq 0 ]; then "$@"; fi
}

if [ "$PROBLEMS" -gt 0 ]; then
    echo >&2
    echo "Not ready to release: $PROBLEMS prerequisite(s) missing (listed above)." >&2
    echo "A release needs an Apple Developer account. See docs/release.md, \"Release\"." >&2
    if [ "$DRY_RUN" -eq 0 ]; then
        exit 1
    fi
    echo >&2
    echo "Dry run: the steps a release would run are printed below with placeholders." >&2
    [ -n "$IDENTITY" ] || IDENTITY="<APPLE_SIGNING_IDENTITY>"
    [ "${#NOTARY_AUTH[@]}" -gt 0 ] || NOTARY_AUTH=(--keychain-profile "<MNEME_NOTARY_PROFILE>")
else
    echo "  ok: signing as $IDENTITY"
    echo "  ok: notarizing with $NOTARY_HOW"
    echo "  ok: entitlements $ENTITLEMENTS"
fi

APP="$BUNDLE_DIR/Mneme.app"
SUBMIT_ZIP="$BUNDLE_DIR/Mneme-notarize.zip"

cd "$ROOT"

# --- 1. Audit ----------------------------------------------------------------
echo
echo "1. Local-only audit"
run "$ROOT/scripts/audit-local-only.sh"

# --- 2. Build and sign -------------------------------------------------------
echo
echo "2. Build and sign (Tauri signs with APPLE_SIGNING_IDENTITY, hardened runtime on)"
run env "${HIDE_FROM_TAURI[@]}" APPLE_SIGNING_IDENTITY="$IDENTITY" \
    pnpm tauri build --config "$RELEASE_CONF"

# --- 3. Check the signature --------------------------------------------------
echo
echo "3. Check the signature"
run codesign --verify --deep --strict --verbose=2 "$APP"
if [ "$DRY_RUN" -eq 0 ]; then
    SIGNATURE="$(codesign --display --verbose=4 "$APP" 2>&1)"
    echo "$SIGNATURE" | grep -q "^Authority=Developer ID Application:" || {
        echo "error: $APP is not signed with a Developer ID Application certificate." >&2
        exit 1
    }
    echo "$SIGNATURE" | grep -q "flags=.*runtime" || {
        echo "error: $APP is signed without the hardened runtime; Apple will not notarize it." >&2
        exit 1
    }
else
    echo "  (then: confirm the authority is Developer ID Application and the hardened runtime flag is set)"
fi

# --- 4. Notarize -------------------------------------------------------------
echo
echo "4. Notarize (uploads the app to Apple and waits for the verdict)"
run ditto -c -k --keepParent "$APP" "$SUBMIT_ZIP"
printf '  $ xcrun notarytool submit %q --wait --output-format json' "$SUBMIT_ZIP"
printf ' %q' "${NOTARY_AUTH[@]}"
printf '\n'
if [ "$DRY_RUN" -eq 0 ]; then
    RESULT="$(xcrun notarytool submit "$SUBMIT_ZIP" --wait --output-format json "${NOTARY_AUTH[@]}")" || {
        echo "error: notarytool failed:" >&2
        echo "$RESULT" >&2
        exit 1
    }
    STATUS="$(echo "$RESULT" | jq -r '.status // "unknown"')"
    SUBMISSION="$(echo "$RESULT" | jq -r '.id // ""')"
    echo "  status: $STATUS (submission $SUBMISSION)"
    if [ "$STATUS" != "Accepted" ]; then
        echo "error: Apple did not accept the app. The notary log follows." >&2
        if [ -n "$SUBMISSION" ]; then
            xcrun notarytool log "$SUBMISSION" "${NOTARY_AUTH[@]}" >&2 || true
        fi
        exit 1
    fi
    rm -f "$SUBMIT_ZIP"
else
    echo "  (then: stop unless the status is Accepted, printing \`xcrun notarytool log\` for the submission)"
fi

# --- 5. Staple and check with Gatekeeper ------------------------------------
echo
echo "5. Staple the ticket and ask Gatekeeper"
run xcrun stapler staple "$APP"
run xcrun stapler validate "$APP"
run spctl --assess --type execute --verbose=4 "$APP"

# --- 6. Package --------------------------------------------------------------
echo
echo "6. Package"
if [ "$DRY_RUN" -eq 0 ]; then
    VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")"
else
    VERSION="<version>"
fi
FINAL_ZIP="$BUNDLE_DIR/Mneme-$VERSION-$(uname -m).zip"
run ditto -c -k --keepParent "$APP" "$FINAL_ZIP"
run shasum -a 256 "$FINAL_ZIP"

echo
if [ "$DRY_RUN" -eq 1 ]; then
    echo "Dry run: nothing was built, signed or sent."
    [ "$PROBLEMS" -eq 0 ] || exit 1
else
    echo "Done: $FINAL_ZIP"
    echo "Copy it to another Mac, unzip, and open Mneme.app to confirm it opens without a warning."
fi
