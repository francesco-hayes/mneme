#!/usr/bin/env bash
# Local-only audit for Mneme.
#
# Fails (exit 1) if anything that can open a network connection is reachable
# from the app outside the model downloader, or if the app's configuration
# lets the webview reach a remote host. Read-only: it runs `cargo tree` with
# `--locked --offline`, `jq` and `grep`, and writes nothing.
#
# The rule it enforces (prds/00-foundation.md, "Local-only rules"):
#   - The only code allowed to open a network connection is the model
#     downloader in crates/mneme-transcribe, and its only HTTP client is ureq.
#   - No updater, analytics or crash-reporting plugin.
#   - The webview's content security policy allows nothing remote.
#
# Usage:
#   scripts/audit-local-only.sh              run every check
#   scripts/audit-local-only.sh --deny NAME  also treat crate NAME as
#                                            network-capable (repeatable); used
#                                            to show that the check can fail
#   scripts/audit-local-only.sh --list       print the denylist and exit
#   scripts/audit-local-only.sh --help
#
# Exit codes: 0 all checks pass, 1 at least one violation, 2 could not run.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# The one workspace crate allowed to reach the network, and the one client
# crate it may use to do so. Everything on the denylist must sit behind both:
# reachable from the app only through DOWNLOADER_CRATE, and inside that crate
# only through HTTP_CLIENT.
DOWNLOADER_CRATE="mneme-transcribe"
HTTP_CLIENT="ureq"

# Crates that open sockets, speak a network protocol, or exist to do TLS.
# Type-only crates are deliberately absent: Tauri itself depends on `http` and
# `url`, which parse and hold values and cannot connect to anything.
DENY_CRATES="
actix-web async-h1 async-io async-native-tls async-net async-std async-tls
async-tungstenite attohttpc axum boring boring-sys c-ares curl curl-sys ehttp
embedded-websocket fastwebsockets git2 h2 h3 hickory-proto hickory-resolver
http-client hyper hyper-rustls hyper-tls hyper-util isahc lettre libcurl-sys
libgit2-sys libp2p libssh2-sys minreq native-tls nng openssl openssl-sys
paho-mqtt quiche quinn quinn-proto quinn-udp reqwest reqwest-middleware rouille
rumqttc rustls rustls-native-certs rustls-platform-verifier rustls-webpki s2n-quic
s2n-tls salvo schannel smol socket2 ssh2 surf tarpc tiny_http tokio-native-tls
tokio-rustls tokio-tungstenite tonic trust-dns-proto trust-dns-resolver
tungstenite ureq ureq-proto warp webpki webpki-roots websocket ws zmq
"

# Crates that are harmless without one feature and network-capable with it.
# Tauri pulls in tokio for its runtime, without `net`.
DENY_FEATURES="tokio:net mio:net"

# Tauri plugins and SDKs that update, phone home, or report crashes. Matched
# as a regular expression against every crate name in the tree and every
# package name in package.json.
DENY_PLUGIN_RE='(^|[/@-])(tauri-plugin-(updater|http|websocket|upload|aptabase|sentry|posthog)|plugin-(updater|http|websocket|upload)|sentry|aptabase|posthog|bugsnag|crashpad|minidump|minidumper|crash-handler|backtrace-io|opentelemetry|mixpanel|amplitude|segment|rudderstack|datadog|logrocket|fullstory|hotjar|plausible|google-analytics|gtag|rollbar|honeybadger|raygun|countly|matomo|self_update|cargo-packager-updater|sparkle|sparkle-sys)([/_-]|$)'

# One line, so it can be handed to awk as a variable.
DENY_CRATES="$(echo "$DENY_CRATES" | tr '\n' ' ')"

EXTRA_DENY=""

usage() {
    sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --deny)
            [ $# -ge 2 ] || { echo "error: --deny needs a crate name" >&2; exit 2; }
            EXTRA_DENY="$EXTRA_DENY $2"
            shift 2
            ;;
        --list)
            echo "Network-capable crates (denied outside $DOWNLOADER_CRATE -> $HTTP_CLIENT):"
            # shellcheck disable=SC2086
            printf '  %s\n' $DENY_CRATES
            echo "Denied crate features:"
            # shellcheck disable=SC2086
            printf '  %s\n' $DENY_FEATURES
            echo "Denied plugin and SDK name pattern:"
            echo "  $DENY_PLUGIN_RE"
            exit 0
            ;;
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
done

for tool in cargo jq; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool is not on PATH; the audit needs it." >&2
        exit 2
    }
done

cd "$ROOT"

FAILURES=0
fail() {
    FAILURES=$((FAILURES + 1))
    echo "  FAIL: $*"
}
pass() {
    echo "  ok:   $*"
}

# Prints "name|features" for every package in a `cargo tree` selection, one per
# line, deduplicated. Normal and build dependencies only: dev-dependencies
# (test servers and the like) are not part of the app. The host target only,
# which is the one Mneme ships for.
tree() {
    local out
    if ! out="$(cargo tree --locked --offline -e normal,build --prefix none \
        --format '{p}|{f}' "$@" 2>&1)"; then
        echo "error: cargo tree failed:" >&2
        echo "$out" >&2
        echo "(the audit runs offline; run \`cargo fetch\` once if crates are missing)" >&2
        exit 2
    fi
    # "name v1.2.3 (/path) (proc-macro)|feat,feat (*)"  ->  "name|feat,feat"
    echo "$out" | awk -F'|' 'NF >= 2 {
        split($1, head, " ")
        feats = $2
        sub(/ \(\*\)$/, "", feats)
        print head[1] "|" feats
    }' | sort -u
}

# Reads "name|features" lines on stdin and prints each one that is on the
# denylist, as "name" or "name (feature X)".
denied() {
    awk -F'|' -v crates="$DENY_CRATES $EXTRA_DENY" -v feats="$DENY_FEATURES" '
        BEGIN {
            n = split(crates, c, /[ \n\t]+/)
            for (i = 1; i <= n; i++) if (c[i] != "") deny[c[i]] = 1
            m = split(feats, f, /[ \n\t]+/)
            for (i = 1; i <= m; i++) if (f[i] != "") {
                split(f[i], pair, ":")
                denyfeat[pair[1]] = pair[2]
            }
        }
        {
            if ($1 in deny) { print $1; next }
            if ($1 in denyfeat) {
                k = split($2, have, ",")
                for (i = 1; i <= k; i++)
                    if (have[i] == denyfeat[$1]) print $1 " (feature " have[i] ")"
            }
        }' | sort -u
}

# The allowed pair must exist as named, or the checks below would be vacuous
# (`cargo tree --prune` of a missing package errors, but be explicit).
ALL="$(tree --workspace)"
echo "$ALL" | grep -q "^$DOWNLOADER_CRATE|" || {
    echo "error: workspace has no crate named $DOWNLOADER_CRATE; update this script." >&2
    exit 2
}

echo "1. Network-capable crates outside the model downloader"
# Everything the other workspace members reach without passing through the
# downloader crate. Features are the ones Cargo unifies for the whole
# selection, so a `net` feature switched on anywhere shows up here.
OUTSIDE="$(tree --workspace --exclude "$DOWNLOADER_CRATE" --prune "$DOWNLOADER_CRATE" | denied)"
if [ -n "$OUTSIDE" ]; then
    while IFS= read -r hit; do
        fail "$hit is reachable without going through $DOWNLOADER_CRATE"
        name="${hit%% *}"
        echo "        who depends on it:"
        cargo tree --locked --offline -e normal,build --workspace \
            --prune "$DOWNLOADER_CRATE" -i "$name" --depth 3 2>&1 | sed 's/^/          /' || true
    done <<EOF
$OUTSIDE
EOF
else
    pass "none"
fi

echo "2. Network-capable crates inside $DOWNLOADER_CRATE but not behind $HTTP_CLIENT"
if echo "$ALL" | grep -q "^$HTTP_CLIENT|"; then
    INSIDE="$(tree -p "$DOWNLOADER_CRATE" --prune "$HTTP_CLIENT" | denied)"
else
    # The client is gone; nothing in the downloader crate may be denied at all.
    INSIDE="$(tree -p "$DOWNLOADER_CRATE" | denied)"
fi
if [ -n "$INSIDE" ]; then
    while IFS= read -r hit; do
        fail "$hit is used by $DOWNLOADER_CRATE directly, not through $HTTP_CLIENT"
    done <<EOF
$INSIDE
EOF
else
    pass "none"
fi

echo "   Network-capable crates in the whole tree (all behind $DOWNLOADER_CRATE -> $HTTP_CLIENT if 1 and 2 pass):"
FOUND="$(echo "$ALL" | denied)"
if [ -n "$FOUND" ]; then
    echo "$ALL" | awk -F'|' 'NR == FNR { want[$1] = 1; next } ($1 in want) { print "        " $1 " [" $2 "]" }' \
        <(echo "$FOUND" | cut -d' ' -f1) -
else
    echo "        (none)"
fi

echo "3. Updater, analytics and crash-reporting plugins"
RUST_PLUGINS="$(echo "$ALL" | cut -d'|' -f1 | grep -E -i "$DENY_PLUGIN_RE" || true)"
if [ -n "$RUST_PLUGINS" ]; then
    for p in $RUST_PLUGINS; do fail "crate $p is in the dependency tree"; done
else
    pass "no such crate in the dependency tree"
fi
JS_PLUGINS="$(jq -r '[(.dependencies // {}), (.devDependencies // {}), (.optionalDependencies // {})]
    | map(keys) | add | .[]' package.json | grep -E -i "$DENY_PLUGIN_RE" || true)"
if [ -n "$JS_PLUGINS" ]; then
    for p in $JS_PLUGINS; do fail "package $p is in package.json"; done
else
    pass "no such package in package.json"
fi

echo "4. Tauri configuration"
# jq program: prints one line per problem in a (merged) Tauri config.
# A CSP source is remote if it names a network scheme or a wildcard. The one
# exception is http://ipc.localhost, Tauri's own channel to the Rust side,
# which never leaves the process.
# shellcheck disable=SC2016  # a jq program, not shell
CONFIG_CHECK='
def tokens:
    if type == "string" then [splits("[ ;]+")]
    elif type == "array" then map(tokens) | add // []
    elif type == "object" then [.[] | tokens] | add // []
    else [] end;
def remote:
    . != "http://ipc.localhost"
    and test("^(https?|wss?|ftp|file):|^//|\\*"; "i");
(.app.security.csp) as $csp
| (if $csp == null then "app.security.csp is missing or null: the webview has no content security policy"
   else ( $csp | tokens | map(select(remote)) | unique | .[] | "CSP allows a remote source: \(.)" ),
        ( if ($csp | type) == "object" and ($csp | has("default-src") | not)
          then "CSP has no default-src" else empty end ),
        ( if ($csp | type) == "string" and ($csp | test("default-src") | not)
          then "CSP has no default-src" else empty end )
   end),
  (if (.app.security.devCsp // null) != null
   then (.app.security.devCsp | tokens | map(select(remote and (startswith("http://localhost") | not))) | unique | .[]
         | "devCsp allows a remote source: \(.)")
   else empty end),
  (if (.plugins // {}) | has("updater") then "plugins.updater is configured" else empty end),
  (if (.bundle.createUpdaterArtifacts // false) != false then "bundle.createUpdaterArtifacts is on" else empty end),
  (if (.build.frontendDist // "" | tostring | test("^https?:"; "i")) then "build.frontendDist is a remote URL" else empty end),
  ((.app.windows // [])[] | select((.url // "") | test("^(https?|wss?):"; "i")) | "window \(.label // "?") loads a remote URL: \(.url)")
'
MAIN_CONF="src-tauri/tauri.conf.json"
check_conf() { # label, then the files to merge in order
    local label="$1" problems
    shift
    problems="$(jq -r -s 'reduce .[] as $c ({}; . * $c) | '"$CONFIG_CHECK" "$@")" || {
        echo "error: could not read $label" >&2
        exit 2
    }
    if [ -n "$problems" ]; then
        while IFS= read -r line; do fail "$label: $line"; done <<EOF
$problems
EOF
    else
        pass "$label"
    fi
}
check_conf "$MAIN_CONF" "$MAIN_CONF"
# Every other config file in src-tauri is merged over the main one by the Tauri
# CLI (platform files automatically, the rest with --config), so check each as
# it would be used.
for overlay in src-tauri/tauri.*.conf.json; do
    [ -e "$overlay" ] || continue
    [ "$overlay" = "$MAIN_CONF" ] && continue
    check_conf "$MAIN_CONF + $overlay" "$MAIN_CONF" "$overlay"
done

CAP_PROBLEMS=""
for cap in src-tauri/capabilities/*.json; do
    [ -e "$cap" ] || continue
    found="$(jq -r --arg f "$cap" '
        (if has("remote") then "\($f): grants commands to remote URLs (\"remote\")" else empty end),
        ((.permissions // [])[] | (if type == "object" then .identifier else . end)
         | select(test("^(http|websocket|updater|upload|shell):"))
         | "\($f): permission \(.)")' "$cap")"
    [ -n "$found" ] && CAP_PROBLEMS="$CAP_PROBLEMS$found
"
done
if [ -n "$CAP_PROBLEMS" ]; then
    while IFS= read -r line; do
        [ -n "$line" ] && fail "$line"
    done <<EOF
$CAP_PROBLEMS
EOF
else
    pass "capabilities grant nothing remote and no network, shell or updater permission"
fi

echo "5. Sockets from the standard library, outside the downloader"
# A dependency audit cannot see std::net, which needs no crate. This is a text
# search of our own sources only; it does not look inside dependencies.
STD_NET="$(grep -rnE 'std::net|TcpStream|TcpListener|UdpSocket|UnixStream|UnixListener|UnixDatagram' \
    --include='*.rs' crates/*/src src-tauri/src 2>/dev/null \
    | grep -v "^crates/$DOWNLOADER_CRATE/src/models" || true)"
if [ -n "$STD_NET" ]; then
    while IFS= read -r line; do fail "socket type named in $line"; done <<EOF
$STD_NET
EOF
else
    pass "no socket types named in workspace sources outside crates/$DOWNLOADER_CRATE/src/models"
fi

echo
if [ "$FAILURES" -gt 0 ]; then
    echo "Local-only audit FAILED: $FAILURES problem(s)."
    exit 1
fi
echo "Local-only audit passed."
