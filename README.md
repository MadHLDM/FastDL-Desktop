# FastDL Desktop

Desktop app for validating, installing, compressing, publishing, and rolling back Sven Co-op FastDL packages.

FastDL Desktop is built with Tauri, Rust, Vite, and TypeScript. The Rust backend handles package validation, file operations, compression, remote publishing, audit logs, rollback safety, and credential storage. The frontend provides the desktop workflow for configuration, validation, publishing, upload history, and log review.

## Features

- Validates `.zip`, `.tar`, `.tar.gz`, `.tgz`, `.tar.bz2`, and `.tbz2` packages before install.
- Blocks unsafe archive paths, including traversal, absolute paths, Windows drive paths, backslashes, unsafe components, reserved Windows names, symlinks, hard links, duplicate paths, and case collisions.
- Enforces per-content limits, folder rules, extension rules, and lowercase paths where required.
- Validates `maps/*.res` entries against the ZIP contents and the configured server root.
- Previews destination conflicts before install.
- Installs package files into the configured game server root.
- Generates optional `.gz` and `.bz2` FastDL files.
- Supports local FastDL output or remote FTP/SFTP publishing.
- Supports optional remote game server publishing over FTP/SFTP.
- Writes upload manifests and audit logs.
- Provides hash-aware rollback for installed files.
- Rolls back published FTP/SFTP files recorded in manifests.
- Shows upload history, manifests, and audit logs inside the app.
- Opens the logs folder from the app.
- Stores FTP/SFTP passwords and SFTP key passphrases in platform secret storage.
- Pins trusted SFTP host fingerprints and blocks SFTP operations if a host key changes.
- Generates release `SHA256SUMS.txt` files for MSI/EXE artifacts.

## Security

FastDL Desktop treats package installation as a sensitive file operation. Archive contents are validated before extraction, rollback checks expected hashes before deleting installed files, and SFTP host fingerprints must be trusted before SFTP publishing can proceed.

Credentials are not saved in the app JSON config. On Windows, secrets are stored in Windows Credential Manager. On Linux, secrets are stored through Secret Service using `secret-tool`.

Unsigned Windows builds may trigger SmartScreen warnings. Verify release downloads with `SHA256SUMS.txt`.

## Requirements

- Node.js and npm.
- Rust toolchain with Cargo in `PATH`.
- Platform Tauri prerequisites for Windows/Linux.
- Linux: `libsecret-tools` and an unlocked Secret Service keyring for saving FTP/SFTP credentials.

## Development

```powershell
npm install
npm run tauri dev
```

## Build

```powershell
npm run tauri build
```

Release builds generate `SHA256SUMS.txt` files next to the bundled MSI/EXE artifacts and a combined checksum file under `src-tauri/target/release/bundle`.

Release signing notes are documented in `docs/RELEASE.md`.

## SFTP Troubleshooting

`SSH handshake failed: Unable to exchange encryption key` means the app reached the SSH server, but no compatible key exchange, cipher, or host key algorithm was negotiated before authentication. FileZilla can still connect to the same server if it supports a different SSH algorithm set than the libssh2 backend used by FastDL Desktop. Newer builds include the local FastDL Desktop SFTP algorithm list in the error message so it can be compared with the FileZilla connection log.

Check the server SFTP algorithm settings and confirm that the endpoint is SFTP over SSH, not FTPS. If the server is managed by a hosting provider, send them the raw FastDL Desktop error and ask whether their SFTP service supports libssh2-compatible modern algorithms.

On the first successful SFTP handshake, Test SFTP shows the server host key fingerprint and asks whether it should be trusted. FastDL Desktop saves that fingerprint and blocks later SFTP operations if the server presents a different host key.

## CI

GitHub Actions checks Windows and Linux with:

- `npm ci`
- `npm run build`
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
- `cargo check --manifest-path src-tauri/Cargo.toml --locked`
- `cargo test --manifest-path src-tauri/Cargo.toml --locked`
