use crate::config::SftpConfig;
use serde::Serialize;
use ssh2::{HashType, HostKeyType, MethodType, Session};
use std::fs::File;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpHostFingerprint {
	pub host: String,
	pub port: u16,
	pub host_key_type: String,
	pub sha256: String,
}

#[derive(Debug, Error)]
pub enum SftpError {
	#[error("SFTP is enabled but host is missing")]
	MissingHost,
	#[error("SFTP is enabled but username is missing")]
	MissingUsername,
	#[error("SFTP is enabled but remote FastDL root is missing")]
	MissingRemoteRoot,
	#[error("SFTP is enabled but neither password nor private key authentication is configured")]
	MissingAuthentication,
	#[error("could not connect to SFTP server: {0}")]
	Connect(#[source] std::io::Error),
	#[error("could not create SSH session")]
	CreateSession,
	#[error("{message}")]
	Handshake {
		message: String,
		#[source]
		source: ssh2::Error,
	},
	#[error("SFTP host key is missing from the SSH session")]
	MissingHostKey,
	#[error("SFTP host is not trusted yet: {host}:{port} {fingerprint}. Use Test SFTP to review and trust this host before publishing.")]
	MissingTrustedHost {
		host: String,
		port: u16,
		fingerprint: String,
	},
	#[error("SFTP host fingerprint changed for {host}:{port}. Expected {expected}, got {actual}. This may indicate a server change or a man-in-the-middle attack.")]
	HostFingerprintChanged {
		host: String,
		port: u16,
		expected: String,
		actual: String,
	},
	#[error("SFTP authentication failed: {0}")]
	Auth(#[source] ssh2::Error),
	#[error("SFTP operation failed: {0}")]
	Sftp(#[source] ssh2::Error),
	#[error("local file operation failed: {0}")]
	Io(#[source] std::io::Error),
}

pub fn publish_files(config: &SftpConfig, files: &[(PathBuf, String)]) -> Result<(), SftpError> {
	if !config.enabled || files.is_empty() {
		return Ok(());
	}
	let (session, remote_root) = connect(config)?;
	let sftp = session.sftp().map_err(SftpError::Sftp)?;
	for (local_path, relative_path) in files {
		let remote_path = remote_join(remote_root, relative_path);
		ensure_remote_parent(&sftp, &remote_path)?;
		let mut local = File::open(local_path).map_err(SftpError::Io)?;
		let mut remote = sftp
			.create(Path::new(&remote_path))
			.map_err(SftpError::Sftp)?;
		std::io::copy(&mut local, &mut remote).map_err(SftpError::Io)?;
	}
	Ok(())
}

pub fn delete_files(config: &SftpConfig, files: &[String]) -> Result<Vec<String>, SftpError> {
	if !config.enabled || files.is_empty() {
		return Ok(Vec::new());
	}
	let (session, remote_root) = connect(config)?;
	let sftp = session.sftp().map_err(SftpError::Sftp)?;
	let mut deleted = Vec::new();
	for relative_path in files {
		let remote_path = remote_join(remote_root, relative_path);
		let path = Path::new(&remote_path);
		if sftp.stat(path).is_err() {
			continue;
		}
		sftp.unlink(path).map_err(SftpError::Sftp)?;
		deleted.push(relative_path.clone());
	}
	Ok(deleted)
}

pub fn test_connection(config: &SftpConfig) -> Result<(), SftpError> {
	let (session, remote_root) = connect_with_optional_root(config)?;
	let sftp = session.sftp().map_err(SftpError::Sftp)?;
	if let Some(remote_root) = remote_root {
		sftp.stat(Path::new(remote_root)).map_err(SftpError::Sftp)?;
	} else {
		sftp.stat(Path::new(".")).map_err(SftpError::Sftp)?;
	}
	Ok(())
}

pub fn inspect_host(config: &SftpConfig) -> Result<SftpHostFingerprint, SftpError> {
	let (session, _, host, port) = handshake(config)?;
	host_fingerprint(&session, host, port)
}

fn connect(config: &SftpConfig) -> Result<(Session, &str), SftpError> {
	let (session, remote_root) = connect_with_optional_root(config)?;
	let remote_root = remote_root.ok_or(SftpError::MissingRemoteRoot)?;
	Ok((session, remote_root))
}

fn connect_with_optional_root(config: &SftpConfig) -> Result<(Session, Option<&str>), SftpError> {
	let username = config
		.username
		.as_deref()
		.ok_or(SftpError::MissingUsername)?;
	let (session, remote_root, host, port) = handshake(config)?;
	verify_trusted_host(config, &session, host, port)?;

	if let Some(private_key_path) = &config.private_key_path {
		session
			.userauth_pubkey_file(
				username,
				None,
				private_key_path,
				config.private_key_passphrase.as_deref(),
			)
			.map_err(SftpError::Auth)?;
	} else if let Some(password) = &config.password {
		session
			.userauth_password(username, password)
			.map_err(SftpError::Auth)?;
	} else {
		return Err(SftpError::MissingAuthentication);
	}

	Ok((session, remote_root))
}

fn handshake(config: &SftpConfig) -> Result<(Session, Option<&str>, &str, u16), SftpError> {
	let host = config.host.as_deref().ok_or(SftpError::MissingHost)?;
	let remote_root = config
		.remote_fastdl_root
		.as_deref()
		.filter(|root| !root.is_empty());
	let address = (host, config.port)
		.to_socket_addrs()
		.map_err(SftpError::Connect)?
		.next()
		.ok_or_else(|| {
			SftpError::Connect(std::io::Error::new(
				std::io::ErrorKind::NotFound,
				"could not resolve SFTP host",
			))
		})?;
	let tcp = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(SftpError::Connect)?;
	tcp.set_read_timeout(Some(IO_TIMEOUT))
		.map_err(SftpError::Connect)?;
	tcp.set_write_timeout(Some(IO_TIMEOUT))
		.map_err(SftpError::Connect)?;
	let mut session = Session::new().map_err(|_| SftpError::CreateSession)?;
	session.set_tcp_stream(tcp);
	session
		.handshake()
		.map_err(|error| handshake_error(error, &session))?;

	Ok((session, remote_root, host, config.port))
}

fn verify_trusted_host(
	config: &SftpConfig,
	session: &Session,
	host: &str,
	port: u16,
) -> Result<(), SftpError> {
	let fingerprint = host_fingerprint(session, host, port)?;
	let Some(expected) = config.trusted_host_fingerprint.as_deref() else {
		return Err(SftpError::MissingTrustedHost {
			host: host.to_string(),
			port,
			fingerprint: fingerprint.sha256,
		});
	};
	if expected.trim() != fingerprint.sha256 {
		return Err(SftpError::HostFingerprintChanged {
			host: host.to_string(),
			port,
			expected: expected.to_string(),
			actual: fingerprint.sha256,
		});
	}
	Ok(())
}

fn host_fingerprint(
	session: &Session,
	host: &str,
	port: u16,
) -> Result<SftpHostFingerprint, SftpError> {
	let (_, host_key_type) = session.host_key().ok_or(SftpError::MissingHostKey)?;
	let hash = session
		.host_key_hash(HashType::Sha256)
		.ok_or(SftpError::MissingHostKey)?;
	Ok(SftpHostFingerprint {
		host: host.to_string(),
		port,
		host_key_type: host_key_type_label(host_key_type).to_string(),
		sha256: format!("SHA256:{}", base64_no_padding(hash)),
	})
}

fn host_key_type_label(host_key_type: HostKeyType) -> &'static str {
	match host_key_type {
		HostKeyType::Rsa => "ssh-rsa",
		HostKeyType::Dss => "ssh-dss",
		HostKeyType::Ecdsa256 => "ecdsa-sha2-nistp256",
		HostKeyType::Ecdsa384 => "ecdsa-sha2-nistp384",
		HostKeyType::Ecdsa521 => "ecdsa-sha2-nistp521",
		HostKeyType::Ed25519 => "ssh-ed25519",
		HostKeyType::Unknown => "unknown",
	}
}

fn base64_no_padding(bytes: &[u8]) -> String {
	const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
	let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
	for chunk in bytes.chunks(3) {
		let first = chunk[0];
		let second = *chunk.get(1).unwrap_or(&0);
		let third = *chunk.get(2).unwrap_or(&0);
		output.push(TABLE[(first >> 2) as usize] as char);
		output.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
		if chunk.len() > 1 {
			output.push(TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
		}
		if chunk.len() > 2 {
			output.push(TABLE[(third & 0b0011_1111) as usize] as char);
		}
	}
	output
}

fn handshake_error(error: ssh2::Error, session: &Session) -> SftpError {
	let raw_message = error.message().to_string();
	let supported = supported_algorithm_summary(session);
	let message = if raw_message
		.to_ascii_lowercase()
		.contains("unable to exchange encryption key")
	{
		format!(
			"SSH handshake failed: no compatible SFTP encryption/key-exchange algorithm was negotiated. FileZilla may connect because it supports a different SSH algorithm set. Compare the server or FileZilla SFTP log against the FastDL Desktop algorithms below. Raw error: {error}. FastDL Desktop supports: {supported}"
		)
	} else {
		format!("SSH handshake failed: {error}. FastDL Desktop supports: {supported}")
	};
	SftpError::Handshake {
		message,
		source: error,
	}
}

fn supported_algorithm_summary(session: &Session) -> String {
	let kex = supported_algorithms(session, MethodType::Kex);
	let host_key = supported_algorithms(session, MethodType::HostKey);
	let client_to_server_cipher = supported_algorithms(session, MethodType::CryptCs);
	let server_to_client_cipher = supported_algorithms(session, MethodType::CryptSc);
	format!(
		"kex=[{}]; host_key=[{}]; cipher_client_to_server=[{}]; cipher_server_to_client=[{}]",
		kex, host_key, client_to_server_cipher, server_to_client_cipher
	)
}

fn supported_algorithms(session: &Session, method_type: MethodType) -> String {
	match session.supported_algs(method_type) {
		Ok(algorithms) if !algorithms.is_empty() => algorithms.join(", "),
		Ok(_) => "none reported".to_string(),
		Err(error) => format!("could not read supported algorithms: {error}"),
	}
}

pub(crate) fn remote_join(root: &str, relative: &str) -> String {
	format!(
		"{}/{}",
		root.trim_end_matches('/'),
		relative.trim_start_matches('/')
	)
}

fn ensure_remote_parent(sftp: &ssh2::Sftp, remote_path: &str) -> Result<(), SftpError> {
	let Some((parent, _)) = remote_path.rsplit_once('/') else {
		return Ok(());
	};
	let mut current = String::new();
	for part in parent.split('/') {
		if part.is_empty() {
			current.push('/');
			continue;
		}
		if current != "/" && !current.is_empty() {
			current.push('/');
		}
		current.push_str(part);
		let path = Path::new(&current);
		if sftp.stat(path).is_err() {
			sftp.mkdir(path, 0o755).map_err(SftpError::Sftp)?;
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{base64_no_padding, remote_join};

	#[cfg(windows)]
	#[test]
	fn windows_build_supports_ecdsa_host_keys() {
		let session = ssh2::Session::new().expect("session should be created");
		let host_keys = session
			.supported_algs(ssh2::MethodType::HostKey)
			.expect("host key algorithms should be reported");
		assert!(
			host_keys
				.iter()
				.any(|algorithm| algorithm.starts_with("ecdsa-sha2-")),
			"Windows SFTP build must include ECDSA host keys; supported host keys: {host_keys:?}"
		);
	}

	#[test]
	fn remote_join_keeps_single_separator() {
		assert_eq!(
			remote_join("/fastdl/", "/maps/test.bsp.gz"),
			"/fastdl/maps/test.bsp.gz"
		);
		assert_eq!(
			remote_join("fastdl", "maps/test.bsp.gz"),
			"fastdl/maps/test.bsp.gz"
		);
	}

	#[test]
	fn base64_fingerprint_encoding_omits_padding() {
		assert_eq!(base64_no_padding(b""), "");
		assert_eq!(base64_no_padding(b"a"), "YQ");
		assert_eq!(base64_no_padding(b"ab"), "YWI");
		assert_eq!(base64_no_padding(b"abc"), "YWJj");
	}
}
