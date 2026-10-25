# GitHub repository and release checklist

Manual steps for publishing Pegoles on GitHub and cutting releases. The
repository is **`zezortdx/Pegoles-Agent`** (display name Pegoles Agent),
created private on 2026-09-27 on a GitHub Free account and made **public**
the same day, once the private-side gates in `docs/RELEASE_GATES.md` had
passed. 0.1.0 was released from it (source-first) on 2026-09-27. Boxes
marked [x] are done and were read back through the GitHub API after the
switch.

Workflows involved (all in `.github/workflows/`):

| File | Runs on | Purpose |
|---|---|---|
| `ci.yml` | PRs, pushes to `main` | the gate (read-only token, no secrets) |
| `codeql.yml` | PRs to `main`, pushes to `main`, weekly | code scanning, advanced setup: results upload to code scanning (the SARIF gate `.github/scripts/codeql-gate.py` also runs) |
| `dependency-review.yml` | PRs to `main` | blocks new high/critical vulnerable dependencies |
| `release.yml` | `v*` tags, only when the variable `PEGOLES_BINARY_RELEASES` is `true` (§5) | `build` (no secrets: gate, unsigned bundle, runtime sandbox tests, manifest + SBOM) → `sign` (protected `release` environment: sign, notarize, staple, verify, checksums; runs no dependency code) → `publish` (attest, draft release) |

## 1. Before the first push

- [x] Repository decided: `zezortdx/Pegoles-Agent`. The issue template
  chooser links private vulnerability reporting, and the README's
  attestation command names the repository.
- [x] `crates/pegoles-computer/catalog/images.json` points at the hosted
  archive (step 6); the `UNPUBLISHED` marker is gone.
- [ ] Optional: claim the npm scope `@pegoles` and the crate names. Nothing
  publishes today (packages are `private: true`, every workspace crate is
  `publish = false`).
- [x] History purged of `scripts/brand/__pycache__/*.pyc` (they held an
  absolute `/Users/<name>/...` path) with `git filter-repo`, before the
  first push. Every other commit, author, date and message is unchanged:
  checked commit by commit, and the release tip tree is identical.
- [x] Commit author email: rewritten to the maintainer's GitHub `noreply`
  address on every commit before the repository went public (the
  maintainer's decision); the history was rescanned afterwards. Squash
  merges are made with the `noreply` address as author.
- [x] Only explicit refs were pushed: `main` and `release/v0.1.0`. Never
  `git push --mirror` or `--all`: local `refs/backup/*` and `refs/codex/*`
  hold superseded snapshots, including the pre-purge history.
- [x] Secret scan over the whole history before the first push: gitleaks
  8.30.1 found only four policy tripwire fixtures (truncated key headers in
  tests). They are listed by exact fingerprint in `.gitleaksignore`. A
  broader scan of every blob for personal paths, hostnames, emails, tokens
  and private artifacts found nothing else. The `secrets` CI job repeats
  the gitleaks scan on every run.

## 2. Create the repository

- [x] Created `zezortdx/Pegoles-Agent` **private**, without
  README/license/gitignore. Actions stayed disabled until both branches
  were pushed, so the old unpinned `ci.yml` on the pre-release `main`
  snapshot never ran.
- [x] Note: on GitHub Free, artifact attestations, dependency review, code
  scanning uploads, secret scanning, rulesets/branch protection and
  environment reviewers work only in **public** repositories, which is
  why everything below was applied right after the switch.
- [x] First GitHub run (PR #1): every CI job and every CodeQL language
  green on head `0744f6b` (details in `docs/RELEASE_GATES.md`, gate 7).
  One full CI + CodeQL run took about 16 macOS runner minutes (billed
  10x) and about 35 Linux/Windows minutes. Public repositories use
  GitHub-hosted runners without a minutes quota; nothing runs on the
  maintainer's Mac.

## 3. Repository settings

Settings -> General

- [x] Visibility: **public** (2026-09-27).
- [x] Features: Issues on; Wiki off; Projects off; Discussions off.
- [x] Pull Requests: **squash merging** only (linear history); merge
  commits and rebase merging off; "Always suggest updating pull request
  branches" on; "Automatically delete head branches" on; **"Allow
  auto-merge" off**. The first PR (`release/v0.1.0`) lands by fast-forward
  instead: `.gitleaksignore` fingerprints name commits, so its history
  must land unchanged.
- [x] Releases: **release immutability** on.

Settings -> Actions -> General

- [x] Actions permissions: selected actions only: GitHub-created actions
  plus `dtolnay/rust-toolchain@*, pnpm/action-setup@*`, Marketplace
  "verified" creators not allowed; **"Require actions to be pinned to a
  full-length commit SHA"** on.
- [x] Fork pull request workflows: **"Require approval for all external
  contributors"**; fork PRs get no write token and no secrets.
- [x] Workflow permissions: **read-only** default token; "Allow GitHub
  Actions to create and approve pull requests" **off**.
- [x] Runners: no self-hosted runners. Pull requests never run on
  self-hosted runners.

Settings -> Rules -> Rulesets -> branch ruleset `main` (id 24074522)

- [x] Enforcement: Active. Target: default branch. Bypass list: empty.
- [x] Restrict deletions; Block force pushes; Require linear history.
- [x] Require a pull request before merging (squash only): required approvals **0**
  while there is a single maintainer (GitHub never lets authors approve
  their own PR; raise to 1 with a second maintainer and enable "Dismiss
  stale approvals" and "Require approval of the most recent push");
  **Require conversation resolution** on.
- [x] Require status checks to pass, "Require branches to be up to date"
  on, these 16 checks:
  - `Rust (macOS arm64)`
  - `Rust release profile (macOS arm64)`
  - `Swift VM helper (macOS arm64)`
  - `Frontend (lint, typecheck, test, build)`
  - `Guest runtime (aarch64 Linux check)`
  - `Portability (ubuntu-24.04)`
  - `Portability (windows-2025)`
  - `Supply chain (cargo-deny, cargo-audit, pnpm audit, lockfiles)`
  - `Workflow and script hygiene`
  - `Secret scan (gitleaks, full history)`
  - `Dependency review`
  - `CodeQL (actions)`, `CodeQL (javascript-typescript)`,
    `CodeQL (python)`, `CodeQL (rust)`, `CodeQL (swift)`
- [x] Require code scanning results: CodeQL, security alerts "High or
  higher", other alerts "Errors".

Settings -> Rules -> Rulesets -> tag ruleset `release tags` (id 24074524)

- [x] Target: tags matching `v*` and `guest-image-*`.
- [x] Restrict creations, updates and deletions; non-fast-forward blocked.
  Bypass list: Repository admin (the maintainer), so only the maintainer
  creates these tags. GitHub applies a bypass to every rule of a ruleset,
  so an admin could still move or delete a tag that has no release; the
  tag of a published release is locked by release immutability, and
  release tags are never moved (`v0.1.0`, `guest-image-0.3`).

Settings -> Environments -> `release` (created)

- [x] Required reviewers: the maintainer. **"Prevent self-review" off** while there is one
  maintainer (otherwise nobody can approve); turn it on when there are
  two. Do not add any secret before this rule exists.
- [x] Deployment branches and tags: "Selected branches and tags" with one
  **tag** rule `v*` and no branch rule (the workflow also refuses non-tag
  refs). "Allow administrators to bypass" off.
- [ ] Environment secrets, only when binary releases start (§4, §5). Today
  there are none, and no repository or organization secrets either:

  | Secret | Value |
  |---|---|
  | `APPLE_CERTIFICATE_P12_BASE64` | base64 of the Developer ID Application `.p12` (step 4) |
  | `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
  | `APPLE_SIGN_IDENTITY` | exactly as `security find-identity -v -p codesigning` prints it, e.g. `Developer ID Application: Name (TEAMID)` |
  | `NOTARY_KEY_ID` | App Store Connect API key ID |
  | `NOTARY_ISSUER_ID` | App Store Connect issuer ID |
  | `NOTARY_KEY_P8_BASE64` | base64 of `AuthKey_<KEYID>.p8` |

  Set them from files, never through shell history, e.g.
  `base64 -i DeveloperID.p12 | gh secret set APPLE_CERTIFICATE_P12_BASE64 --env release`.

Settings -> Code security

- [x] Dependency graph and Dependabot alerts: on. The one alert so far
  (glib 0.18 `VariantStrIter` unsoundness) is dismissed as not used: glib
  is only in Tauri's Linux GTK tree and absent from the macOS dependency
  graph, the same reasoning as `deny.toml`. Dependabot cannot update it
  (Tauri pins gtk-rs 0.18), so its security-update job for it fails; that
  is expected until Tauri moves on.
- [x] Dependabot security updates: on. Version updates come from
  `.github/dependabot.yml` (no auto-merge); major-version upgrades wait
  until after 0.1.0.
- [x] Code scanning: **advanced setup** (`codeql.yml`), five languages
  uploading; default setup stays off (they conflict).
- [x] Secret scanning: on; **Push protection: on**. The `secrets` CI job
  also scans the full history on every run.
- [x] **Private vulnerability reporting: on** (SECURITY.md and
  CODE_OF_CONDUCT.md route reports there).

## 4. Signing and notarization credentials

Developer ID Application certificate (Apple Developer Program, Account
Holder role):

1. Keychain Access -> Certificate Assistant -> Request a Certificate From
   a Certificate Authority -> "Saved to disk" (creates the private key in
   the login keychain).
2. developer.apple.com -> Certificates -> "+" -> **Developer ID
   Application** (G2 Sub-CA) -> upload the CSR -> download and open the
   `.cer`.
3. Keychain Access -> My Certificates -> "Developer ID Application: Name
   (TEAMID)" (with its private key) -> Export -> `.p12` with a strong
   password.
4. `security find-identity -v -p codesigning` shows the exact identity
   string for `APPLE_SIGN_IDENTITY`.
5. Put the `.p12` into the `release` environment (step 3), keep an offline
   backup, and delete the file (`*.p12` is gitignored, but never leave it
   in the checkout).

Notarization key (App Store Connect API):

1. App Store Connect -> Users and Access -> Integrations -> App Store
   Connect API -> Team Keys -> "+", access **Developer**.
2. Download `AuthKey_<KEYID>.p8` (possible once), note the Key ID and the
   Issuer ID shown on that page.
3. Store the three values in the `release` environment (step 3).
4. For local notarization instead:
   `xcrun notarytool store-credentials pegoles-notary --key AuthKey_<KEYID>.p8 --key-id <KEYID> --issuer <ISSUER>`,
   then `PEGOLES_SIGN_IDENTITY=... NOTARY_KEYCHAIN_PROFILE=pegoles-notary bash scripts/release/notarize.sh`.

Rotate both if a machine that held them is lost; revoke the certificate
at developer.apple.com and the key in App Store Connect.

## 5. Binary release workflow (deferred)

0.1.x is source-first: `release.yml` runs only when the repository
variable `PEGOLES_BINARY_RELEASES` is `true`. Before enabling it:

- [ ] Signing and notary credentials in the `release` environment (§4),
  with its required reviewer.
- [ ] The release `build` job runs `scripts/local-model/build-runtime.sh`,
  whose smoke test requires Metal. Confirm that GitHub's hosted `macos-15`
  arm64 runner passes it; if it does not, decide explicitly (e.g. a
  dedicated runner used only by the protected `release` environment, never
  by pull requests); do not weaken the smoke test silently.
- [x] The Xcode path pinned in the workflows (`/Applications/Xcode_26.2.app`)
  exists on the `macos-15` arm64 image. When it is retired, bump it in
  `ci.yml`, `codeql.yml` and `release.yml` together.

## 6. Guest image hosting

The app downloads exactly the image pinned in
`crates/pegoles-computer/catalog/images.json` (archive SHA-256, disk
SHA-512).

- [x] Release `guest-image-0.3` (tag on `main`), with
  `pegoles-base-0.3-arm64.raw.gz` uploaded as a draft, its digest checked
  (GitHub reports the pinned SHA-256), then published (immutable, not
  "latest").
- [x] The catalog URL is that asset. It downloads anonymously; the
  installed app downloaded and verified it in 37 s in the 0.1.0 acceptance
  run.
- A new image is a new release `guest-image-<version>` and new pins; an
  existing image release is never edited.

## 7. Cutting a source-first release

1. Set the version in all four places:
   `apps/desktop/src-tauri/tauri.conf.json`,
   `apps/desktop/src-tauri/Cargo.toml` (then `cargo metadata` to refresh
   `Cargo.lock`), `apps/desktop/package.json`, `package.json`.
2. Move the `Unreleased` entries in `CHANGELOG.md` under the version
   with today's date.
3. Merge through a PR with every required check green.
4. On the merge commit: a fresh `git clone`, `./scripts/install.sh`,
   then the acceptance run in the installed app (fresh data: set up
   Pegoles Local and the computer from their real sources, a task, Stop,
   Reset, quit and relaunch).
5. Tag that exact commit and push only the tag:
   `git tag -a v<version> -m "Pegoles Agent <version>" <commit> && git push origin v<version>`.
6. From the clean-clone build of the tag, generate `SHA256SUMS`, the
   CycloneDX SBOM and `build-manifest.json`
   (`scripts/release/provenance.sh --no-dmg`, then `shasum -a 256` over
   the attached files, plus the pinned guest image archive's line, into
   `SHA256SUMS`), and create a **draft**
   release for the tag with them attached; the source archives GitHub
   generates are the primary artifact. No DMG is attached unless it is
   clearly labeled as unsigned and not notarized.
7. Review the draft, then **publish** it. With release immutability on,
   the tag and assets are frozen from then on.
8. A bad release is never edited in place: publish a new version.

0.1.0 followed these steps: merge commit `58616be` (PR #10), tag object
`157ae56`, release
<https://github.com/zezortdx/Pegoles-Agent/releases/tag/v0.1.0> (immutable,
latest) with `build-manifest.json`, `pegoles-0.1.0.cdx.json` and
`SHA256SUMS`; the release workflow was skipped for the tag, as intended.
