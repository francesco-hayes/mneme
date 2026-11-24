# 035: A new version on main builds the Mac download into a draft release

Status: done · PRD: [06](../prds/06-integration-and-finish.md) · Lane: release · Depends on: 025 · Owns: `.github/workflows/release.yml`, `src-tauri/tauri.github-release.conf.json`, `docs/release.md`, root `README.md`

## Goal

When Francesco pushes a new version to `main`, the files someone needs to install Mneme are built and put in a draft GitHub release, which he then publishes. No tag is made by hand. Asked for on 2026-11-24.

Decided the same day:

- **Mac only.** Windows and Linux were asked about, but system audio capture (the Them side) and Move to Trash are macOS-only, so those builds would record the microphone only. Porting system audio is its own piece of work.
- **Unsigned for now.** There is no Apple Developer account. The app is signed ad hoc and not notarized; users approve it once with Open Anyway. Developer ID signing can be added later with repository secrets.

## Acceptance criteria

- [x] A workflow runs on every push to `main` and builds only when `v<version>` has no tag or release yet. It then tags the commit and creates a draft release with the files. By hand it rebuilds an existing tag's files. It fails when `Cargo.toml` and `package.json` disagree on the version.
- [x] `pnpm bump <version>` sets the version in `Cargo.toml`, `package.json` and `Cargo.lock`.
- [x] It runs the local-only audit, builds the app and a disk image, checks the app is validly signed ad hoc, and attaches `Mneme-<version>-macos-arm64.dmg`, `Mneme-<version>-macos-arm64.zip` and `SHA256SUMS.txt` to the release.
- [x] Actions are pinned to commit hashes; the job's token can write release contents only.
- [x] The README has install steps, including Open Anyway, and a Releasing section with a flow diagram (`docs/release-flow.svg`, light and dark); `docs/release.md` says how to make a release and what the workflow does.
- [x] `actionlint` passes; the audit passes with the new overlay; the same build runs locally.

## Manual checks

- [ ] `pnpm bump` to a new version, push to `main`, and watch the workflow finish with a tag and a draft release holding the three files; publish it.
- [ ] Push a change that does not touch the version: the run stops after the check job.
- [ ] On another Apple Silicon Mac, or this one after deleting the local build: download the DMG in a browser, drag to Applications, open, use Open Anyway, record a short meeting.

## Result

**Built**

- `.github/workflows/release.yml`: a check job, then one job on `macos-15` (Apple Silicon). Checks out the commit, installs Node 24, pnpm from `packageManager` and stable Rust, checks the tag against the version, runs `scripts/audit-local-only.sh`, builds, verifies the signature, and attaches the DMG, a zip of the app and `SHA256SUMS.txt`: `gh release create --draft` when no release exists for the tag, `gh release upload --clobber` when one does. `workflow_dispatch` with a `tag` input rebuilds an existing release. `permissions: contents: write`; checkout does not keep the token.
- `src-tauri/tauri.github-release.conf.json`: targets `app` and `dmg`, ad hoc signing (`signingIdentity: "-"`), hardened runtime off, no updater artifacts.
- README "Install" section; `docs/release.md` "GitHub release" section and a version check in "Before a release".

**Checks (2026-11-24, on this Mac)**

- `actionlint` passes. The local-only audit passes with the new overlay merged in.
- `CI=true pnpm tauri build --config src-tauri/tauri.github-release.conf.json` built `Mneme.app` (16.9 MiB) and `Mneme_0.1.0_aarch64.dmg` (5.4 MiB). `codesign --verify --deep --strict` passes, `Signature=adhoc`. `hdiutil verify` passes; the mounted DMG holds `Mneme.app` (signature valid) and an Applications link. The built `Info.plist` has version 0.1.0, minimum macOS 14.4, and both permission strings.

**Changed the same day:** the first version ran on `release: published`. Francesco published v0.1.0 and the release sat with no files while the build ran, so the trigger moved to the tag push and the workflow makes a draft. GitHub does not run workflows for a draft being created, so the tag has to be what starts it.

**Changed again the same day:** Francesco asked not to make tags at all. The workflow now runs on every push to `main`: a check job on Linux reads the version and stops unless `v<version>` is new, then the Mac job builds, creates the tag through the API with the workflow token (so no other workflow starts, and publishing the draft makes no new tag), and creates the draft. The tag-push trigger was removed. `scripts/bump-version.sh` (`pnpm bump`) was added and tried on a throwaway worktree: 0.1.0 to 0.1.1 changed `Cargo.toml`, `package.json` and six crates in `Cargo.lock`, `cargo metadata --locked` agreed, the same version again exits 1, `banana` exits 2. The flow diagram was checked rendered in light and dark.

**Not verified**

- The workflow has not run on GitHub. Action versions and commit hashes were looked up on GitHub the same day; the runner's tool versions (cmake, jq, Xcode) are assumed from GitHub's macOS 15 image.
- The downloaded, quarantined app has not been opened through Open Anyway, and its permission prompts have not been seen in that state.

**Follow-ups**

- Developer ID signing and notarization in the workflow once there is an Apple Developer account (secrets for the certificate and notary key).
- Intel Mac build (`x86_64-apple-darwin` or universal), if wanted.
- Windows and Linux: needs system audio capture for each (WASAPI loopback, PipeWire/PulseAudio monitor), a Trash implementation, and whisper-rs built without the `metal` feature there.
