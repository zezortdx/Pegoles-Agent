# WinGet (draft)

Manifests for `winget install Pegoles.PegolesAgent`, **not submitted**.
They become submittable once a Windows release exists: a published GitHub
release carrying `Pegoles-Setup-x64.exe` (built by
`scripts/package-windows.sh`) that passed the Windows gates in
`docs/RELEASE_GATES.md`.

To fill them for a release `vX.Y.Z`:

1. Replace `VERSION` with `X.Y.Z` in the three files.
2. Replace `INSTALLER_SHA256` with the installer's SHA-256 (the
   `.sha256` file next to it; `winget hash Pegoles-Setup-x64.exe` gives
   the same value).
3. Validate: `winget validate --manifest packaging/winget/manifests`, then
   install from the local manifest on a clean Windows 11 PC:
   `winget install --manifest packaging/winget/manifests`.
4. Submit with `wingetcreate submit` to microsoft/winget-pkgs (the
   repository owner's decision and account).

Notes:

- WinGet accepts unsigned installers, but SmartScreen and Smart App
  Control still apply to what WinGet downloads; signing is tracked in
  `docs/RELEASE_GATES.md`.
- `Scope: machine` and `elevationRequired`: the installer is per machine
  because it registers the `PegolesVmBroker` service, whose binary must
  live where only administrators can write.
- The package identifier and publisher name are placeholders for the
  owner to confirm before the first submission.
