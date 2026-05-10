# FastDL Desktop

Desktop app for validating and publishing Sven Co-op FastDL packages.

This project is intentionally split in two layers:

- `src-tauri/src`: Rust core and Tauri commands.
- `src`: GUI built with Vite and TypeScript.

The first implemented slice validates ZIP packages with the same security posture as the Discord bot:

- rejects path traversal, absolute paths, Windows drive paths, backslashes, unsafe components, Windows reserved names, symlinks, duplicate paths, and case collisions;
- applies per-content limits and folder/extension rules;
- enforces lowercase paths when content rules require it;
- validates `maps/*.res` resources against the ZIP contents and the server root;
- previews compressed FastDL output and destination conflicts.
- keeps validation as a standalone validate-only command before install;
- installs packages into the server root while optionally generating `.gz` and `.bz2` copies in a separate FastDL root;
- writes upload manifests, supports hash-aware rollback, and appends audit logs under `.fastdl-desktop/logs/audit.tsv`;
- optionally publishes original game server files over FTP/SFTP while retaining local validation/staging;
- optionally publishes generated FastDL files over FTP or SFTP and records remote publish history;
- rolls back FTP/SFTP published files when remote publishing is enabled during rollback;
- shows audit log entries and upload manifests inside the desktop app, with a shortcut to open the logs folder;
- pins trusted SFTP host fingerprints after user confirmation and blocks SFTP publishing if a host key changes;
- persists desktop configuration while keeping FTP/SFTP passwords and SFTP key passphrases in Windows Credential Manager instead of the config file.

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
- `cargo check --manifest-path src-tauri/Cargo.toml --locked`

## Porting Roadmap

1. Port install/staging/rollback manifests from the Python bot.
2. Add gzip/bzip2 generation.
3. Add SFTP publishing.
4. Add release signing and update-channel automation.
5. Add integration tests with fixture ZIPs.
