# GitHub repository and release checklist

Manual steps for publishing Pegoles on GitHub and cutting releases. The
repository has no remote yet and its owner/name are undecided: wherever
this document says `<owner>/<repo>`, use the real values once they exist.
Nothing in the tree hard-codes them except the placeholders listed in
step 1.

Workflows involved (all in `.github/workflows/`):

| File | Runs on | Purpose |
|---|---|---|
| `ci.yml` | PRs, pushes to `main` | the gate (read-only token, no secrets) |
| `codeql.yml` | PRs to `main`, pushes to `main`, weekly | code scanning, advanced setup |
| `dependency-review.yml` | PRs to `main` | blocks new high/critical vulnerable dependencies |
| `release.yml` | `v*` tags only | `build` (no secrets: gate, unsigned bundle, runtime sandbox tests, manifest + SBOM) → `sign` (protected `release` environment: sign, notarize, staple, verify, checksums; runs no dependency code) → `publish` (attest, draft release) |

## 1. Before the first push

- [ ] Decide `<owner>/<repo>`. Then replace the placeholders:
  - `crates/pegoles-computer/catalog/images.json`: the `UNPUBLISHED` owner
    in the guest image URL (the release workflow refuses to build while it
    is there; see step 6);
  - optionally `.github/ISSUE_TEMPLATE/config.yml`: the commented
    security-report link (`https://github.com/<owner>/<repo>/security/advisories/new`).
- [ ] Claim or rename what the code refers to: the GitHub owner, the npm
  scope `@pegoles` (packages are `private: true`, so nothing publishes),
  and the crate names (every workspace crate is `publish = false`).
- [ ] Old history contains `scripts/brand/__pycache__/*.pyc` (removed from
  the tree) with an absolute `/Users/<name>/...` path inside. It reveals
  the same name as the commit author email. Either accept that, or purge
  both before the first push (`git filter-repo --path-glob
  'scripts/brand/__pycache__/*' --invert-paths` together with the email
  rewrite below). Rewriting is only cheap before anything is pushed.
- [ ] Commit author email: every commit carries the maintainer's personal
  address. Before the first push is the only cheap moment to rewrite it to
  the GitHub `noreply` address; at least set
  `git config user.email <id>+<user>@users.noreply.github.com` for new
  commits.
- [ ] Push only explicit refs: `git push -u origin main` and, later,
  individual tags. Never `git push --mirror` or `--all`: local
  `refs/backup/*` and `refs/codex/*` hold superseded snapshots.
- [ ] Run a secret scan over the history once more (e.g. `gitleaks git .`).

## 2. Create the repository

- [ ] Create `<owner>/<repo>` **private**, without README/license/gitignore
  (they exist in the tree). Push `main`.
- [ ] Note: on GitHub Free/Pro/Team, artifact attestations, dependency
  review and code scanning uploads work only in **public** repositories.
  While the repository is private, expect `codeql.yml` and
  `dependency-review.yml` to fail and do not run `release.yml` (its
  attestation step would fail). Flip to public (step 3) before cutting a
  release.

## 3. Repository settings

Settings -> General

- [ ] Visibility: public once steps 1-5 are done (Danger Zone -> Change visibility).
- [ ] Features: Issues on; Wiki off; Projects off (unless used); Discussions optional.
- [ ] Pull Requests: allow **squash merging** only (linear history); turn
  off merge commits and rebase merging; "Always suggest updating pull
  request branches" on; "Automatically delete head branches" on;
  **"Allow auto-merge" off**.
- [ ] Releases: **"Enable release immutability"** on (published releases
  and their tags and assets can no longer change; drafts stay editable).

Settings -> Actions -> General

- [ ] Actions permissions: "Allow `<owner>`, and select non-`<owner>`,
  actions and reusable workflows"; allow GitHub-created actions and this
  list: `dtolnay/rust-toolchain@*, pnpm/action-setup@*`. Turn on
  **"Require actions to be pinned to a full-length commit SHA"**.
- [ ] Fork pull request workflows: **"Require approval for all external
  contributors"**.
- [ ] Workflow permissions: **"Read repository contents and packages
  permissions"** (read-only default token); **uncheck "Allow GitHub
  Actions to create and approve pull requests"**.
- [ ] Runners: no self-hosted runners. Pull requests never run on
  self-hosted runners.

Settings -> Rules -> Rulesets -> New branch ruleset `main`

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
  - `Dependency review`
  - `CodeQL (actions)`, `CodeQL (javascript-typescript)`,
    `CodeQL (python)`, `CodeQL (rust)`, `CodeQL (swift)`
- [ ] Require code scanning results: CodeQL, alerts "High or higher".

Settings -> Rules -> Rulesets -> New tag ruleset `release tags`

- [ ] Target: tags matching `v*` and `guest-image-*`.
- [ ] Restrict creations, updates and deletions; bypass list: Repository
  admin (the maintainer) for creations only. Tags are then created only by
  the maintainer and never moved or deleted.

Settings -> Environments -> New environment `release`

- [ ] Required reviewers: the maintainer. Leave **"Prevent self-review"
  off** while there is one maintainer (otherwise nobody can approve); turn
  it on when there are two.
- [ ] Deployment branches and tags: "Selected branches and tags", add a
  **tag** rule `v*` and no branch rule (the workflow also refuses non-tag refs).
- [ ] Environment secrets (and no repository or organization secrets):

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

- [ ] Dependency graph: on. Dependabot alerts: on. Dependabot security
  updates: on; grouped security updates: on. Version updates come from
  `.github/dependabot.yml` (no auto-merge).
- [ ] Code scanning: keep **advanced setup** (`codeql.yml`); do not enable
  default setup (they conflict). Copilot Autofix optional.
- [ ] Secret scanning: on; **Push protection: on**; validity checks and
  non-provider patterns on where offered.
- [ ] **Private vulnerability reporting: on** (SECURITY.md and
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

## 5. First release dry run (repository public, before announcing)

- [ ] The release `build` job runs `scripts/local-model/build-runtime.sh`
  (via `package-macos.sh build`), whose smoke test requires Metal on the
  build machine. Confirm on the first run
  that GitHub's hosted `macos-15` arm64 runner passes it. If it does not,
  decide explicitly (e.g. a dedicated runner used only by the protected
  `release` environment, never by pull requests); do not weaken the smoke
  test silently.
- [ ] Confirm the Xcode path pinned in the workflows
  (`/Applications/Xcode_26.2.app`) still exists on the `macos-15` image;
  bump it in `ci.yml`, `codeql.yml` and `release.yml` together.

## 6. Guest image hosting

The app downloads exactly the image pinned in
`crates/pegoles-computer/catalog/images.json` (archive SHA-256, disk
SHA-512).

- [ ] Create a release `guest-image-0.3` (tag on `main`), upload
  `pegoles-base-0.3-arm64.raw.gz`, and check its SHA-256 against the
  catalog before publishing it.
- [ ] Replace `UNPUBLISHED` in the catalog URL with `<owner>/<repo>` and
  merge that change before tagging an app release.

## 7. Cutting a release

1. [ ] Set the version (e.g. `0.1.0-rc.1`) in all four places:
   `apps/desktop/src-tauri/tauri.conf.json`,
   `apps/desktop/src-tauri/Cargo.toml` (then `cargo metadata` to refresh
   `Cargo.lock`), `apps/desktop/package.json`, `package.json`. The release
   workflow refuses a tag that differs from any of them. After the build,
   check the bundle version macOS shows for a pre-release version.
2. [ ] Move the `Unreleased` entries in `CHANGELOG.md` under the version
   with today's date.
3. [ ] Merge through a PR with every required check green.
4. [ ] Tag the merge commit on `main` and push only that tag:
   `git tag -a v0.1.0-rc.1 -m "Pegoles Agent 0.1.0-rc.1" && git push origin v0.1.0-rc.1`.
5. [ ] Actions -> release -> approve the `release` deployment.
6. [ ] Review the **draft** release:
   - download every asset; `shasum -a 256 -c SHA256SUMS`;
   - `gh attestation verify Pegoles_<version>_arm64.dmg --repo <owner>/<repo>`;
   - `spctl --assess --type open --context context:primary-signature -v <dmg>`
     and `xcrun stapler validate <dmg>`;
   - `bash scripts/release/verify-artifact.sh <dmg> --distribution`;
   - read `build-manifest.json` (commit = the tag, toolchains, guest image,
     default model) and skim the SBOM;
   - install on a clean Mac (or a fresh macOS user), let it download the
     guest image and the default model, and run a task with Pegoles Local.
7. [ ] Edit the release notes if needed and **publish** the draft (manual;
   the workflow never publishes). With release immutability on, the tag
   and assets are frozen from then on. Mark `-rc` versions as pre-releases
   (the workflow already does).
8. [ ] A bad release is never edited in place: publish a new version.
