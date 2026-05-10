use crate::config::FtpConfig;
use std::fs::File;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::time::Duration;
use suppaftp::{FtpError, FtpStream, Status};
use thiserror::Error;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum FtpPublishError {
	#[error("FTP is enabled but host is missing")]
	MissingHost,
	#[error("FTP is enabled but username is missing")]
	MissingUsername,
	#[error("FTP is enabled but password is missing")]
	MissingPassword,
	#[error("FTP is enabled but remote FastDL root is missing")]
	MissingRemoteRoot,
	#[error("FTP operation failed: {0}")]
	Ftp(#[from] FtpError),
	#[error("local file operation failed: {0}")]
	Io(#[from] std::io::Error),
}

pub fn publish_files(
	config: &FtpConfig,
	files: &[(PathBuf, String)],
) -> Result<(), FtpPublishError> {
	if !config.enabled || files.is_empty() {
		return Ok(());
	}

	let host = config.host.as_deref().ok_or(FtpPublishError::MissingHost)?;
	let username = config
		.username
		.as_deref()
		.ok_or(FtpPublishError::MissingUsername)?;
	let password = config
		.password
		.as_deref()
		.ok_or(FtpPublishError::MissingPassword)?;
	let remote_root = config
		.remote_fastdl_root
		.as_deref()
		.ok_or(FtpPublishError::MissingRemoteRoot)?;
	let mut ftp = FtpStream::connect((host, config.port))?;
	ftp.login(username, password)?;
	ensure_remote_dir(&mut ftp, remote_root)?;
	ftp.cwd(remote_root)?;

	for (local_path, relative_path) in files {
		let normalized = relative_path.replace('\\', "/");
		if let Some((parent, filename)) = normalized.rsplit_once('/') {
			ensure_remote_dir(&mut ftp, parent)?;
			ftp.cwd(parent)?;
			let mut local = File::open(local_path)?;
			ftp.put_file(filename, &mut local)?;
			return_to_root(&mut ftp, parent)?;
		} else {
			let mut local = File::open(local_path)?;
			ftp.put_file(&normalized, &mut local)?;
		}
	}

	ftp.quit()?;
	Ok(())
}

pub fn delete_files(config: &FtpConfig, files: &[String]) -> Result<Vec<String>, FtpPublishError> {
	if !config.enabled || files.is_empty() {
		return Ok(Vec::new());
	}

	let remote_root = config
		.remote_fastdl_root
		.as_deref()
		.ok_or(FtpPublishError::MissingRemoteRoot)?;
	let mut ftp = connect(config)?;
	ensure_remote_dir(&mut ftp, remote_root)?;
	ftp.cwd(remote_root)?;

	let mut deleted = Vec::new();
	for relative_path in files {
		let normalized = relative_path.replace('\\', "/");
		match ftp.rm(&normalized) {
			Ok(()) => deleted.push(normalized),
			Err(FtpError::UnexpectedResponse(response))
				if response.status == Status::FileUnavailable => {}
			Err(error) => return Err(FtpPublishError::Ftp(error)),
		}
	}

	ftp.quit()?;
	Ok(deleted)
}

pub fn test_connection(config: &FtpConfig) -> Result<(), FtpPublishError> {
	let mut ftp = connect(config)?;
	if let Some(remote_root) = config
		.remote_fastdl_root
		.as_deref()
		.filter(|root| !root.is_empty())
	{
		ftp.cwd(remote_root)?;
	} else {
		ftp.pwd()?;
	}
	ftp.quit()?;
	Ok(())
}

fn connect(config: &FtpConfig) -> Result<FtpStream, FtpPublishError> {
	let host = config.host.as_deref().ok_or(FtpPublishError::MissingHost)?;
	let username = config
		.username
		.as_deref()
		.ok_or(FtpPublishError::MissingUsername)?;
	let password = config
		.password
		.as_deref()
		.ok_or(FtpPublishError::MissingPassword)?;
	let address = (host, config.port)
		.to_socket_addrs()?
		.next()
		.ok_or_else(|| {
			std::io::Error::new(std::io::ErrorKind::NotFound, "could not resolve FTP host")
		})?;
	let mut ftp = FtpStream::connect_timeout(address, CONNECT_TIMEOUT)?;
	ftp.get_ref().set_read_timeout(Some(IO_TIMEOUT))?;
	ftp.get_ref().set_write_timeout(Some(IO_TIMEOUT))?;
	ftp.login(username, password)?;
	Ok(ftp)
}

fn ensure_remote_dir(ftp: &mut FtpStream, path: &str) -> Result<(), FtpPublishError> {
	let current = ftp.pwd()?;
	let absolute = path.starts_with('/');
	if absolute {
		ftp.cwd("/")?;
	}

	for part in path.split('/').filter(|part| !part.is_empty()) {
		if ftp.cwd(part).is_err() {
			ftp.mkdir(part)?;
			ftp.cwd(part)?;
		}
	}

	ftp.cwd(&current)?;
	Ok(())
}

fn return_to_root(ftp: &mut FtpStream, parent: &str) -> Result<(), FtpPublishError> {
	for _ in parent.split('/').filter(|part| !part.is_empty()) {
		ftp.cdup()?;
	}
	Ok(())
}
