# Building, signing and releasing Mneme

**A notarized release cannot be made yet.** It needs a paid Apple Developer account, which does not exist for this project at the time of writing (2026-11-12). The release script and its config are written and checked in dry-run mode only; they have never signed anything or contacted Apple. The dev-signing script has also not been run: creating the keychain identity is a step for the person who owns the Mac. Everything else on this page (unsigned builds, the local-only audit) works today.

Contents: [Build](#build) · [Dev signing](#dev-signing) · [Release](#release) · [Local-only audit](#local-only-audit) · [Wi-Fi-off test](#wi-fi-off-test) · [Before a release](#before-a-release) · [Manual test list](#manual-test-list)

## Build

Requirements are in the [README](../README.md#requirements). From the repository root:

```sh
pnpm install
pnpm tauri build --debug    # target/debug/bundle/macos/Mneme.app
pnpm tauri build            # target/release/bundle/macos/Mneme.app
```

Neither command needs a certificate. The result is signed ad hoc by the linker, which is enough to run it on the Mac that built it. Its signature changes with every rebuild, so macOS forgets its microphone and system-audio permission each time. Dev signing fixes that.

Signing settings are kept out of `src-tauri/tauri.conf.json`. They live in two overlay files that the Tauri CLI merges over the main config when given with `--config`:

| File                                    | Used by              | Sets                                                         |
| --------------------------------------- | -------------------- | ------------------------------------------------------------ |
| `src-tauri/tauri.dev-signing.conf.json` | dev builds, by hand  | signing identity `Mneme Dev Signing`, hardened runtime off   |
| `src-tauri/tauri.release.conf.json`     | `scripts/release.sh` | hardened runtime on, `app` bundle only, no updater artifacts |

Neither file holds a credential. The Developer ID identity is never written to a file; it comes from the environment.

## Dev signing

macOS ties a permission to the app's code signature. A build signed with one stable certificate keeps its permissions across rebuilds.

### Set up, once

```sh
scripts/dev-signing.sh            # prints what it would do; changes nothing
scripts/dev-signing.sh --apply    # creates the certificate
```

With `--apply` the script generates a self-signed certificate named `Mneme Dev Signing` (code signing only, valid ten years) and imports it with its key into your login keychain. The key is marked non-extractable and only `/usr/bin/codesign` is allowed to use it. The temporary files are deleted. It changes no trust settings and touches no other keychain item.

If a code-signing identity with that name already exists, the script says so and exits without doing anything. If a certificate with that name exists but is not a usable identity, it stops with an error. It never modifies or deletes.

### Sign a dev build

```sh
pnpm tauri build --debug --config src-tauri/tauri.dev-signing.conf.json
```

Or, without the overlay, set the identity in the environment:

```sh
APPLE_SIGNING_IDENTITY="Mneme Dev Signing" pnpm tauri build --debug
```

The two differ in one way. The overlay also turns the hardened runtime off, so the build can use the microphone without an entitlements file. With only the environment variable, the hardened runtime stays on (Tauri's default) and the microphone works only once the entitlements file from issue 024 is in the config.

The first signing shows a keychain prompt for the new key; choose **Always Allow**. The first signed build asks for microphone and system-audio permission once, because its identity is new. Later rebuilds signed the same way do not ask again.

Check what a build is signed with:

```sh
codesign -dv --verbose=4 target/debug/bundle/macos/Mneme.app   # Authority=Mneme Dev Signing
codesign -d -r- target/debug/bundle/macos/Mneme.app            # requirement names the certificate, not a cdhash
```

### Notes

- `pnpm tauri dev` runs the bare binary, not a signed bundle, so this does not apply to it. Test permission behavior with a signed debug build.
- A self-signed certificate is listed as not trusted (`CSSMERR_TP_NOT_TRUSTED`). `codesign` is expected to sign with it anyway. If it refuses with "no identity found", mark the certificate as trusted for code signing in Keychain Access (open the certificate, Trust, Code Signing: Always Trust) and try again.
- To start the permission prompts over: `tccutil reset All app.mneme.desktop`.
- To remove the certificate: `security delete-identity -c "Mneme Dev Signing" ~/Library/Keychains/login.keychain-db`.
- A dev-signed build only runs without warnings on the Mac that holds the certificate. It is not a release.

## Release

### What you need first

1. Membership of the Apple Developer Program.
2. A **Developer ID Application** certificate in your keychain. Create it at developer.apple.com (Certificates, IDs & Profiles) or in Xcode (Settings, Accounts, Manage Certificates). Confirm with `security find-identity -v -p codesigning`.
3. Credentials for Apple's notary service, stored once as a keychain profile:

   ```sh
   xcrun notarytool store-credentials "mneme-notary" --apple-id "you@example.com" --team-id "ABCDE12345"
   ```

   It asks for an app-specific password (made at account.apple.com) and saves it in the keychain. An App Store Connect API key works too: pass `--key`, `--key-id` and `--issuer` to the same command, or give the key to the script directly (below). Keep the `.p8` file outside the repository.

4. The entitlements file and `Info.plist` usage strings from issue 024, referenced from the Tauri config. The script refuses to run without an entitlements file, because a hardened-runtime app without the audio-input entitlement cannot record.

### Run it

```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (ABCDE12345)"
export MNEME_NOTARY_PROFILE="mneme-notary"

scripts/release.sh --dry-run   # checks prerequisites, prints every step, does nothing
scripts/release.sh
```

Set these in your shell or a file outside the repository, never in a tracked file. Instead of `MNEME_NOTARY_PROFILE` the script accepts `APPLE_API_KEY_PATH`, `APPLE_API_KEY` and `APPLE_API_ISSUER`. It does not accept an Apple ID password from the environment: `notarytool` would need it on its command line, where other processes can read it. Store it in a keychain profile instead.

If anything is missing the script lists what and exits with status 1 before building.

### What the script does

1. Runs the [local-only audit](#local-only-audit) and stops if it fails.
2. Builds with `pnpm tauri build --config src-tauri/tauri.release.conf.json`. Tauri signs the app with `APPLE_SIGNING_IDENTITY` and the hardened runtime. The notary variables are hidden from this step so that Tauri does not notarize on its own; the script does it, in one place, with checks.
3. Verifies the signature: valid, made by a Developer ID Application certificate, hardened runtime flag set.
4. Zips the app and submits it with `xcrun notarytool submit --wait`. If Apple's answer is not `Accepted`, it prints the notary log and stops.
5. Staples the ticket to the app (`xcrun stapler staple`), validates it, and asks Gatekeeper (`spctl --assess`).
6. Writes `target/release/bundle/macos/Mneme-<version>-arm64.zip` and prints its SHA-256.

Then copy the zip to another Mac, unzip it, and open the app. It should open without a Gatekeeper warning, also with that Mac offline (the stapled ticket makes that possible).

Notarization uploads the app to Apple. It is the one step in this project that sends anything anywhere, it happens on the developer's machine at release time, and the shipped app is not involved.

## Local-only audit

```sh
scripts/audit-local-only.sh
```

It is read-only and runs offline. It exits 0 when everything passes and 1 on any violation. It checks:

1. **Dependencies outside the downloader.** No crate on its denylist of network-capable crates (HTTP clients and servers, TLS stacks, WebSocket, QUIC, DNS, raw sockets, and `tokio` or `mio` with the `net` feature) may be reachable from any workspace crate without passing through `mneme-transcribe`.
2. **Dependencies inside the downloader.** Within `mneme-transcribe`, such crates may only be reached through `ureq`, the one allowed HTTP client.
3. **Plugins.** No updater, analytics or crash-reporting crate in the dependency tree, and no such package in `package.json`.
4. **Tauri configuration.** The content security policy exists, has a `default-src`, and names no remote source (`http://ipc.localhost` and `ipc:` are Tauri's in-process channel and are allowed). No updater config, no window or frontend loaded from a URL. Each overlay file is checked merged over the main config. Capabilities grant nothing to remote URLs and no network, shell or updater permission.
5. **Our own sources.** No socket type from the standard library (`std::net`, `TcpStream` and so on) is named in workspace code outside `crates/mneme-transcribe/src/models`.

`scripts/audit-local-only.sh --list` prints the denylist. To see the check fail, name a crate that is present as if it were a network crate:

```sh
scripts/audit-local-only.sh --deny hound    # fails: hound is used outside the downloader
```

State on 2026-11-12: it passes. The network-capable crates in the tree are `ureq`, `ureq-proto`, `rustls`, `rustls-webpki` and `webpki-roots`, all behind `mneme-transcribe` and `ureq`. Tauri brings `http` and `url` (types only, not on the denylist) and `tokio` without `net`.

What it cannot see:

- A dependency that opens sockets through the standard library without any crate on the denylist. Check 5 covers only this repository's sources.
- A network crate whose name is not on the list. Look at the names when adding a dependency.
- Dependencies for other platforms or behind features that are off. It audits the build for this Mac's target, which is the only one shipped.
- What the code does at run time. That is what the Wi-Fi-off test is for.

## Wi-Fi-off test

Do this with a built app (`pnpm tauri build`), not the dev server.

- [ ] While online: launch the app, download a model, quit.
- [ ] Turn Wi-Fi off. Unplug Ethernet and any tethered phone. Confirm: `ping -c 1 -t 2 1.1.1.1` fails.
- [ ] Launch the app. It opens normally: no error, no waiting.
- [ ] Start a meeting. Speak, play some audio from another app, type a note. Transcript lines appear.
- [ ] Stop. The final transcript is there and readable.
- [ ] Quit and relaunch. The meeting is on Home with its notes and transcript.
- [ ] Open Settings. The model list shows without hanging. Starting another download fails with a clear message, and the app keeps working.
- [ ] Turn Wi-Fi back on. The download now works.

Optional, with the network on: while a meeting records, `lsof -nP -i -a -p "$(pgrep -i -x mneme)"` should print nothing. It should list a connection only while a model is downloading.

## Before a release

- [ ] `cargo test --workspace`, `pnpm test`, `pnpm lint` pass.
- [ ] `scripts/audit-local-only.sh` passes.
- [ ] Look at `ureq` again. Version 3.5.2 was published on 2026-11-12, the same day it was adopted here, so it had no track record. Check its changelog and advisories, and whether a later version is the better pin.
- [ ] Decide the two open points from issue 007: whether download redirects should be limited to Hugging Face hosts, and whether proxy or system trust store support is wanted.
- [ ] The Wi-Fi-off test above.
- [ ] The manual test list below.

## Manual test list

Gathered on 2026-11-12 from the "Manual checks" section of every issue file, in the wording used there. Boxes are unticked here on purpose: this is the list to walk through for a release. Where the issue file already records a check as done, that is noted.

**Not finished when this list was written:** 009 (live session, todo), 020 (first run, todo), 021 (Tauri commands, todo), 022 (menu bar icon, todo), 023 (global shortcut, todo), 024 (permissions flow, todo). 008 (benchmark CLI) was paused. Their checks below are copied from the issue files as they stood and may change when those issues are implemented; re-read the issue files before a release. Until 021 is done the built app has no working backend, so every check that says "in the built app" cannot be run yet.

Issues with no manual checks: 001, 005, 006, 007, 010 to 020, 026.

### Audio capture

002 Mic capture

- [ ] Run the ignored live test or a small example; speak; the WAV plays back clean at the right speed.
- [ ] Plug in or switch the input device mid-recording; capture continues.

003 System audio taps

- [ ] Play audio from another app; the ignored live test or example records it cleanly. (Recorded as measured on 2026-11-12 in the issue; not yet listened to by ear.)
- [ ] Mneme's own sounds are not captured.
- [ ] Switch output device mid-capture; capture continues.
- [ ] After the process exits, Audio MIDI Setup shows no leftover aggregate device. (Recorded as checked on 2026-11-12 with `system_profiler SPAudioDataType`, not in Audio MIDI Setup itself.)

004 Two-channel recorder

- [ ] During a real Zoom call with headphones: `mic.wav` has only me, `system.wav` has only the other side, both clean.
- [ ] The two files line up when played together.

### Transcription

008 Benchmark CLI (paused)

- [ ] Run `mneme-bench` on a real meeting recording (from `mneme-record`) for each model size I care about, and confirm the table and the recommendation.

### Live pipeline

009 Live session (todo)

- [ ] A 30 minute real call gives a full, ordered transcript at stop, with Me and Them correct (with headphones).

### The app

021 Tauri commands (todo)

- [ ] In the built app: first run downloads a model; New meeting records; notes save; Stop gives a transcript; the meeting appears on Home; relaunch keeps everything.
- [ ] Force-quit mid-meeting, relaunch: the meeting is listed as interrupted with its transcript.

022 Menu bar icon (todo)

- [ ] Icon changes on start and stop from the window, the menu, and after a crash recovery launch.
- [ ] Timer ticks in the menu bar; readable in light and dark menu bars.
- [ ] Start from the menu bar with the window closed records, and Open Mneme shows the live meeting.

023 Global shortcut (todo)

- [ ] Set a shortcut in Settings; from another app it starts and stops recording.
- [ ] Clear it; the combination no longer does anything.
- [ ] Pick a combination macOS reserves; the error shows and the old one still works.

024 Permissions flow (todo)

- [ ] On a fresh user account (or after resetting permissions with `tccutil reset` for the bundle id): first run prompts for each permission once, with the right text.
- [ ] Deny the mic, then follow the app's instructions to fix it.
- [ ] Deny system audio: a meeting records only Me lines and says why.

### Signing and release

025 Signing, notarization, local-only audit

- [ ] Run the dev-signing script; rebuild twice; macOS does not re-ask for mic or system audio permission.
- [ ] With Wi-Fi off and a model installed: record, stop, read the transcript. (The [Wi-Fi-off test](#wi-fi-off-test) above.)
- [ ] Once a Developer ID exists: the release script produces a notarized app that opens on another Mac.
