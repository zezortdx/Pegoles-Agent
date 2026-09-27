# GitHub repository and release checklist

Manual steps for publishing Pegoles on GitHub and cutting releases. The
repository is **`zezortdx/Pegoles-Agent`** (display name Pegoles Agent),
created **private** on 2026-09-27 on a GitHub Free account. It stays private
until this checklist and `docs/RELEASE_GATES.md` are complete. Boxes marked
[x] are done. "Plan-blocked" means GitHub Free does not offer the setting on
a private repository; the API refused it, and it has to be applied right
after the switch to public (step 3).

Workflows involved (all in `.github/workflows/`):

| File | Runs on | Purpose |
|---|---|---|
| `ci.yml` | PRs, pushes to `main` | the gate (read-only token, no secrets) |
| `codeql.yml` | PRs to `main`, pushes to `main`, weekly | code scanning, advanced setup (private repository: SARIF artifact + `.github/scripts/codeql-gate.py`) |
| `dependency-review.yml` | PRs to `main` | blocks new high/critical vulnerable dependencies (skipped while private: unsupported there) |
| `release.yml` | `v*` tags only | `build` (no secrets: gate, unsigned bundle, runtime sandbox tests, manifest + SBOM) → `sign` (protected `release` environment: sign, notarize, staple, verify, checksums; runs no dependency code) → `publish` (attest, draft release) |

## 1. Before the first push

- [x] Repository decided: `zezortdx/Pegoles-Agent`. The issue template
  chooser links private vulnerability reporting, and the README's
  attestation command names the repository.
- [ ] `crates/pegoles-computer/catalog/images.json` keeps the `UNPUBLISHED`
  marker until the archive is actually hosted (step 6). The release
  workflow refuses to build while it is there.
- [ ] Claim or rename what the code refers to: the npm scope `@pegoles`
  (packages are `private: true`, so nothing publishes) and the crate names
  (every workspace crate is `publish = false`).
- [x] History purged of `scripts/brand/__pycache__/*.pyc` (they held an
  absolute `/Users/<name>/...` path) with `git filter-repo`, before the
  first push. Every other commit, author, date and message is unchanged:
  checked commit by commit, and the release tip tree is identical.
- [ ] Commit author email: every commit carries the maintainer's personal
  address, and it becomes public with the repository. Keep it, or rewrite
  it to the GitHub `noreply` address while the repository is still
  private. Rewriting now needs a force push of both branches, so it is the
  maintainer's call. For new commits, at least consider
  `git config user.email <id>+<user>@users.noreply.github.com`.
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
  environment reviewers work only in **public** repositories. While
  private, `codeql.yml` gates on its own SARIF, `dependency-review.yml` is
  skipped, and `release.yml` must not run (its attestation step would
  fail). Flip to public (step 3) before cutting a release.
- [x] First GitHub run (PR #1): every CI job and every CodeQL language
  green on head `0744f6b` (details in `docs/RELEASE_GATES.md`, gate 7).
  One full CI + CodeQL run took about 16 macOS runner minutes (billed
  10x) and about 35 Linux/Windows minutes. A private repository on GitHub
  Free has 2,000 included minutes a month, so weekly Dependabot PRs can
  use up the quota while private; public repositories are not metered.

## 3. Repository settings

Settings -> General

- [ ] Visibility: public once steps 1-5 are done (Danger Zone -> Change
  visibility). Then immediately apply every plan-blocked item below.
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
- [x] Fork pull request workflows (private repository): not run at all;
  no write tokens and no secrets for fork PRs. After going public: set
  **"Require approval for all external contributors"**.
- [x] Workflow permissions: **read-only** default token; "Allow GitHub
  Actions to create and approve pull requests" **off**.
- [x] Runners: no self-hosted runners. Pull requests never run on
  self-hosted runners.

Settings -> Rules -> Rulesets -> New branch ruleset `main` (**plan-blocked
while private**: "Upgrade to GitHub Pro or make this repository public";
classic branch protection is refused the same way)

- [ ] Enforcement: Active. Target: default branch. Bypass list: empty.
- [ ] Restrict deletions; Block force pushes; Require linear history.
- [ ] Require a pull request before merging: required approvals **0**
  while there is a single maintainer (GitHub never lets authors approve
  their own PR; raise to 1 with a second maintainer and enable "Dismiss
  stale approvals" and "Require approval of the most recent push");
  **Require conversation resolution** on.
- [ ] Require status checks to pass, "Require branches to be up to date"
  on. Add these checks (they appear in the picker after they have run
  once, so open a trivial PR first):
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
- [ ] Require code scanning results: CodeQL, alerts "High or higher".

Settings -> Rules -> Rulesets -> New tag ruleset `release tags`
(**plan-blocked while private**)

- [ ] Target: tags matching `v*` and `guest-image-*`.
- [ ] Restrict creations, updates and deletions; bypass list: Repository
  admin (the maintainer) for creations only. Tags are then created only by
  the maintainer and never moved or deleted.

Settings -> Environments -> `release` (created)

- [ ] Required reviewers: the maintainer (**plan-blocked while private**:
  "ensure the billing plan supports the required reviewers protection
  rule"). Leave **"Prevent self-review" off** while there is one
  maintainer (otherwise nobody can approve); turn it on when there are
  two. Do not add any secret before this rule exists.
- [x] Deployment branches and tags: "Selected branches and tags" with one
  **tag** rule `v*` and no branch rule (the workflow also refuses non-tag
  refs). "Allow administrators to bypass" off. GitHub documents deployment
  branch rules for private repositories as a Pro feature; the API accepted
  them here, but their enforcement while private is unverified, so treat
  them as effective only once public.
- [ ] Environment secrets (and no repository or organization secrets;
  none exist today):

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
  (glib 0.18 `VariantStrIter` unsoundness, Linux GTK tree only) is
  dismissed as not used, with the same reasoning as `deny.toml`.
- [ ] Dependabot security updates: on once `main` holds the release branch
  (they target the default branch; paused until then). Version updates
  come from `.github/dependabot.yml` on `main` (no auto-merge).
- [ ] Code scanning: keep **advanced setup** (`codeql.yml`); do not enable
  default setup (they conflict). Uploads start working once public.
- [ ] Secret scanning: on; **Push protection: on** (**plan-blocked while
  private**: "Secret scanning is not available for this repository"; the
  `secrets` CI job scans the full history meanwhile).
- [ ] **Private vulnerability reporting: on** (public repositories only;
  SECURITY.md and CODE_OF_CONDUCT.md route reports there).

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
- [x] The catalog URL is that asset. It downloads anonymously once the
  repository is public.
- A new image is a new release `guest-image-<version>` and new pins; an
  existing image release is never edited.

## 7. Cutting a source-first release

1. [ ] Set the version in all four places:
   `apps/desktop/src-tauri/tauri.conf.json`,
   `apps/desktop/src-tauri/Cargo.toml` (then `cargo metadata` to refresh
   `Cargo.lock`), `apps/desktop/package.json`, `package.json`.
2. [ ] Move the `Unreleased` entries in `CHANGELOG.md` under the version
   with today's date.
3. [ ] Merge through a PR with every required check green.
4. [ ] On the merge commit: a fresh `git clone`, `./scripts/install.sh`,
   then the acceptance run in the installed app (fresh data: set up
   Pegoles Local and the computer from their real sources, a task, Stop,
   Reset, quit and relaunch).
5. [ ] Tag that exact commit and push only the tag:
   `git tag -a v<version> -m "Pegoles Agent <version>" <commit> && git push origin v<version>`.
6. [ ] From the clean-clone build of the tag, generate `SHA256SUMS`, the
   CycloneDX SBOM and `build-manifest.json`
   (`scripts/release/provenance.sh --no-dmg`, then `shasum -a 256` over
   the attached files into `SHA256SUMS`), and create a **draft**
   release for the tag with them attached; the source archives GitHub
   generates are the primary artifact. No DMG is attached unless it is
   clearly labeled as unsigned and not notarized.
7. [ ] Review the draft, then **publish** it. With release immutability on,
   the tag and assets are frozen from then on.
8. [ ] A bad release is never edited in place: publish a new version.
