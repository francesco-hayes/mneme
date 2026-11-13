#!/usr/bin/env bash
# Creates, once, a self-signed code-signing certificate in your login keychain
# so that Mneme dev builds are always signed by the same identity.
#
# Why: macOS ties microphone and system-audio permission to an app's code
# signature. An unsigned (ad-hoc) build has a new signature after every
# rebuild, so macOS asks again each time. A build signed with one stable
# certificate keeps its permissions.
#
# By default this script changes nothing: it checks the keychain (read-only)
# and prints what it would do. Run it with --apply to create the certificate.
# It never modifies or deletes anything, and it stops if a certificate or
# identity with the same name already exists.
#
# Usage:
#   scripts/dev-signing.sh            print the plan, change nothing
#   scripts/dev-signing.sh --apply    create the certificate and key
#   scripts/dev-signing.sh --help
#
# Environment:
#   MNEME_DEV_IDENTITY   certificate name (default "Mneme Dev Signing"). If you
#                        change it, sign with APPLE_SIGNING_IDENTITY instead of
#                        the overlay config, which names the default.
#
# After it has run, sign a dev build with:
#   pnpm tauri build --debug --config src-tauri/tauri.dev-signing.conf.json
# See docs/release.md, "Dev signing".

set -euo pipefail

NAME="${MNEME_DEV_IDENTITY:-Mneme Dev Signing}"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
DAYS=3650
# The system LibreSSL, on purpose: it writes PKCS#12 files in the older format
# that `security import` reads. A Homebrew OpenSSL 3 on PATH writes a newer
# one that `security import` rejects with "MAC verification failed".
OPENSSL=/usr/bin/openssl
APPLY=0

usage() {
    sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --apply) APPLY=1 ;;
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

[ "$(uname -s)" = "Darwin" ] || { echo "error: this script is for macOS." >&2; exit 2; }
for tool in security codesign "$OPENSSL"; do
    command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool not found." >&2; exit 2; }
done
case "$NAME" in
    *[!A-Za-z0-9\ ._-]* | "")
        echo "error: MNEME_DEV_IDENTITY may hold only letters, digits, spaces, dots, underscores and hyphens." >&2
        exit 2
        ;;
esac
[ -f "$KEYCHAIN" ] || { echo "error: login keychain not found at $KEYCHAIN" >&2; exit 2; }

# --- Never touch an existing identity -------------------------------------
# Read-only lookups across every keychain on the search list. `-c` matches the
# name as a substring, so a longer name containing ours also stops the script.
existing_cert="$(security find-certificate -a -c "$NAME" 2>/dev/null | grep '"alis"' || true)"
existing_identity="$(security find-identity -p codesigning 2>/dev/null | grep -F "\"$NAME\"" | sort -u || true)"

if [ -n "$existing_identity" ]; then
    echo "A code-signing identity named \"$NAME\" already exists:"
    echo "$existing_identity"
    echo "Nothing to do; it was left untouched. Sign dev builds with it as described in docs/release.md."
    exit 0
fi
if [ -n "$existing_cert" ]; then
    echo "error: a certificate named \"$NAME\" already exists in a keychain, but it is not" >&2
    echo "a usable code-signing identity (no private key, or not for code signing)." >&2
    echo "This script will not modify or replace it. Inspect it in Keychain Access, or" >&2
    echo "choose another name with MNEME_DEV_IDENTITY." >&2
    exit 1
fi

# --- The plan ---------------------------------------------------------------
cat <<EOF
No certificate or identity named "$NAME" exists yet.

Plan:
  1. Generate a 2048-bit RSA key and a self-signed certificate
       subject:   CN=$NAME
       valid for: $DAYS days
       usage:     code signing only (not a CA)
     in a private temporary folder, with $OPENSSL.
  2. Pack both into a PKCS#12 file protected by a random one-time password.
  3. Import that file into the login keychain
       $KEYCHAIN
     with the key marked non-extractable and /usr/bin/codesign allowed to use it:
       security import <tmp>/identity.p12 -k "$KEYCHAIN" -P <one-time password> -x -T /usr/bin/codesign
  4. Delete the temporary folder (key, certificate, PKCS#12 file).
  5. List the new identity with: security find-identity -p codesigning

Nothing else is changed: no trust settings, no other keychain items, no files
in this repository. To undo it later:
  security delete-identity -c "$NAME" "$KEYCHAIN"
EOF

if [ "$APPLY" -ne 1 ]; then
    echo
    echo "Dry run: nothing was created. Run again with --apply to do it."
    exit 0
fi

# --- Apply ------------------------------------------------------------------
echo
echo "Creating \"$NAME\"..."
umask 077
WORK="$(mktemp -d "${TMPDIR:-/tmp}/mneme-dev-signing.XXXXXX")"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

cat >"$WORK/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
EOF

"$OPENSSL" req -x509 -newkey rsa:2048 -nodes -days "$DAYS" -config "$WORK/cert.cnf" \
    -keyout "$WORK/key.pem" -out "$WORK/cert.pem" 2>/dev/null

P12_PASS="$("$OPENSSL" rand -hex 16)"
"$OPENSSL" pkcs12 -export -inkey "$WORK/key.pem" -in "$WORK/cert.pem" -name "$NAME" \
    -out "$WORK/identity.p12" -passout "pass:$P12_PASS"

security import "$WORK/identity.p12" -k "$KEYCHAIN" -P "$P12_PASS" -x -T /usr/bin/codesign

echo
echo "Done. Identities for code signing named \"$NAME\":"
security find-identity -p codesigning "$KEYCHAIN" | grep -F "\"$NAME\"" | sort -u || {
    echo "error: the import reported success but the identity is not listed." >&2
    exit 1
}
cat <<EOF

macOS lists a self-signed certificate as not trusted (CSSMERR_TP_NOT_TRUSTED).
That is expected and codesign can still sign with it.

Next:
  pnpm tauri build --debug --config src-tauri/tauri.dev-signing.conf.json
The first signing shows a keychain prompt for the new key: choose Always Allow.
EOF
