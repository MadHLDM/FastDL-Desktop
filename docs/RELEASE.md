# Release Checklist

FastDL Desktop is built with Tauri and currently produces Windows MSI and NSIS bundles.

## Required Checks

Run these commands before publishing a release:

```powershell
cargo test --locked
cargo check --locked
cargo fmt --check
npm.cmd run build
npm audit
npm.cmd run tauri -- build
```

## Linux Runtime Requirements

Linux builds use Secret Service through `secret-tool` for FTP/SFTP credentials.

Install the runtime helper on Debian/Ubuntu:

```bash
sudo apt-get install libsecret-tools
```

The user's keyring must be available and unlocked before saving credentials.

If the Tauri build cannot find Cargo from npm, add Cargo to the current shell path:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
```

## Windows Signing

Production releases should be signed with an Authenticode code-signing certificate.

Recommended release requirements:

- Use a certificate issued to the project owner or publisher.
- Keep certificate material out of the repository.
- Store certificate secrets only in the release environment.
- Timestamp signatures so installers remain trusted after certificate expiration.
- Verify signatures before distributing generated installers.

Suggested local signing command after `npm.cmd run tauri -- build`:

```powershell
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a "src-tauri\target\release\bundle\nsis\FastDL Desktop_0.1.0_x64-setup.exe"
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a "src-tauri\target\release\bundle\msi\FastDL Desktop_0.1.0_x64_en-US.msi"
```

Verify signatures:

```powershell
signtool verify /pa /v "src-tauri\target\release\bundle\nsis\FastDL Desktop_0.1.0_x64-setup.exe"
signtool verify /pa /v "src-tauri\target\release\bundle\msi\FastDL Desktop_0.1.0_x64_en-US.msi"
```
