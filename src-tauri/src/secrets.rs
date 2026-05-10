use thiserror::Error;

const SERVICE_PREFIX: &str = "FastDL Desktop";

#[derive(Debug, Error)]
pub enum SecretError {
	#[cfg(not(any(windows, target_os = "linux")))]
	#[error("secure secret storage is not available on this platform")]
	Unavailable,
	#[cfg(target_os = "linux")]
	#[error("Linux Secret Service is unavailable; install libsecret-tools and make sure the keyring is unlocked")]
	LinuxUnavailable,
	#[cfg(target_os = "linux")]
	#[error("Linux Secret Service operation failed: {0}")]
	Linux(String),
	#[cfg(windows)]
	#[error("Windows Credential Manager operation failed with code {0}")]
	Windows(u32),
	#[cfg(windows)]
	#[error("secret is too large for Windows Credential Manager")]
	TooLarge,
}

pub fn read_secret(name: &str) -> Result<Option<String>, SecretError> {
	platform::read_secret(&target_name(name))
}

pub fn write_secret(name: &str, value: Option<&str>) -> Result<(), SecretError> {
	let target = target_name(name);
	match value {
		Some(value) if !value.is_empty() => platform::write_secret(&target, value),
		_ => platform::delete_secret(&target),
	}
}

fn target_name(name: &str) -> String {
	format!("{SERVICE_PREFIX}/{name}")
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
	use super::SecretError;

	pub fn read_secret(_target: &str) -> Result<Option<String>, SecretError> {
		Ok(None)
	}

	pub fn write_secret(_target: &str, _value: &str) -> Result<(), SecretError> {
		Err(SecretError::Unavailable)
	}

	pub fn delete_secret(_target: &str) -> Result<(), SecretError> {
		Ok(())
	}
}

#[cfg(target_os = "linux")]
mod platform {
	use super::{SecretError, SERVICE_PREFIX};
	use std::io::Write;
	use std::process::{Command, Stdio};

	const MISSING_EXIT_CODE: i32 = 1;

	pub fn read_secret(target: &str) -> Result<Option<String>, SecretError> {
		let output = match base_command()
			.arg("lookup")
			.arg("service")
			.arg(SERVICE_PREFIX)
			.arg("account")
			.arg(target)
			.output()
		{
			Ok(output) => output,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
			Err(error) => return Err(SecretError::Linux(error.to_string())),
		};
		if output.status.success() {
			return Ok(Some(
				String::from_utf8_lossy(&output.stdout)
					.trim_end_matches(['\r', '\n'])
					.to_string(),
			));
		}
		if output.status.code() == Some(MISSING_EXIT_CODE) {
			return Ok(None);
		}
		Err(SecretError::Linux(command_error(&output.stderr)))
	}

	pub fn write_secret(target: &str, value: &str) -> Result<(), SecretError> {
		let mut child = match base_command()
			.arg("store")
			.arg("--label")
			.arg(format!("{SERVICE_PREFIX} {target}"))
			.arg("service")
			.arg(SERVICE_PREFIX)
			.arg("account")
			.arg(target)
			.stdin(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
		{
			Ok(child) => child,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
				return Err(SecretError::LinuxUnavailable);
			}
			Err(error) => return Err(SecretError::Linux(error.to_string())),
		};

		let Some(mut stdin) = child.stdin.take() else {
			return Err(SecretError::Linux(
				"could not open secret-tool stdin".to_string(),
			));
		};
		stdin
			.write_all(value.as_bytes())
			.map_err(|error| SecretError::Linux(error.to_string()))?;
		drop(stdin);

		let output = child
			.wait_with_output()
			.map_err(|error| SecretError::Linux(error.to_string()))?;
		if output.status.success() {
			Ok(())
		} else {
			Err(SecretError::Linux(command_error(&output.stderr)))
		}
	}

	pub fn delete_secret(target: &str) -> Result<(), SecretError> {
		let output = match base_command()
			.arg("clear")
			.arg("service")
			.arg(SERVICE_PREFIX)
			.arg("account")
			.arg(target)
			.output()
		{
			Ok(output) => output,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
			Err(error) => return Err(SecretError::Linux(error.to_string())),
		};
		if output.status.success() || output.status.code() == Some(MISSING_EXIT_CODE) {
			Ok(())
		} else {
			Err(SecretError::Linux(command_error(&output.stderr)))
		}
	}

	fn base_command() -> Command {
		Command::new("secret-tool")
	}

	fn command_error(stderr: &[u8]) -> String {
		let message = String::from_utf8_lossy(stderr).trim().to_string();
		if message.is_empty() {
			"secret-tool returned an error".to_string()
		} else {
			message
		}
	}
}

#[cfg(windows)]
mod platform {
	use super::SecretError;
	use std::ffi::OsStr;
	use std::iter;
	use std::os::windows::ffi::OsStrExt;
	use std::ptr;
	use std::slice;
	use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
	use windows_sys::Win32::Security::Credentials::{
		CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
		CRED_TYPE_GENERIC,
	};

	pub fn read_secret(target: &str) -> Result<Option<String>, SecretError> {
		let target = wide(target);
		let mut credential = ptr::null_mut();
		let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
		if ok == 0 {
			let error = unsafe { GetLastError() };
			if error == ERROR_NOT_FOUND {
				return Ok(None);
			}
			return Err(SecretError::Windows(error));
		}

		let secret = unsafe {
			let credential_ref = &*credential;
			let bytes = slice::from_raw_parts(
				credential_ref.CredentialBlob,
				credential_ref.CredentialBlobSize as usize,
			);
			let value = String::from_utf8_lossy(bytes).to_string();
			CredFree(credential.cast());
			value
		};
		Ok(Some(secret))
	}

	pub fn write_secret(target: &str, value: &str) -> Result<(), SecretError> {
		let target = wide(target);
		let username = wide("FastDL Desktop");
		let mut blob = value.as_bytes().to_vec();
		let blob_size = u32::try_from(blob.len()).map_err(|_| SecretError::TooLarge)?;
		let credential = CREDENTIALW {
			Type: CRED_TYPE_GENERIC,
			TargetName: target.as_ptr().cast_mut(),
			CredentialBlobSize: blob_size,
			CredentialBlob: blob.as_mut_ptr(),
			Persist: CRED_PERSIST_LOCAL_MACHINE,
			UserName: username.as_ptr().cast_mut(),
			..Default::default()
		};
		let ok = unsafe { CredWriteW(&credential, 0) };
		if ok == 0 {
			return Err(SecretError::Windows(unsafe { GetLastError() }));
		}
		Ok(())
	}

	pub fn delete_secret(target: &str) -> Result<(), SecretError> {
		let target = wide(target);
		let ok = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
		if ok == 0 {
			let error = unsafe { GetLastError() };
			if error == ERROR_NOT_FOUND {
				return Ok(());
			}
			return Err(SecretError::Windows(error));
		}
		Ok(())
	}

	fn wide(value: &str) -> Vec<u16> {
		OsStr::new(value)
			.encode_wide()
			.chain(iter::once(0))
			.collect()
	}
}
