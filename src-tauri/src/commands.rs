use crate::config::{AppConfig, FtpConfig, SftpConfig, StorageConfig};
use crate::ftp::test_connection as test_ftp;
use crate::secrets::{read_secret, write_secret};
use crate::sftp::{
	inspect_host as inspect_sftp_host_inner, test_connection as test_sftp, SftpHostFingerprint,
};
use crate::storage::{
	install_package_with_progress as install_zip, list_uploads as list_manifests,
	rollback_upload as rollback_manifest, InstallReport, RollbackReport, UploadManifest,
};
use crate::validation::{validate_zip, ValidationReport};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tauri::{async_runtime, AppHandle, Emitter, Manager};

const CONFIG_FILE: &str = "config.json";
const FTP_PASSWORD_SECRET: &str = "ftp-password";
const SFTP_PASSWORD_SECRET: &str = "sftp-password";
const SFTP_KEY_PASSPHRASE_SECRET: &str = "sftp-key-passphrase";
const SERVER_FTP_PASSWORD_SECRET: &str = "server-ftp-password";
const SERVER_SFTP_PASSWORD_SECRET: &str = "server-sftp-password";
const SERVER_SFTP_KEY_PASSPHRASE_SECRET: &str = "server-sftp-key-passphrase";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedConfig {
	storage: StorageConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
	timestamp: String,
	action: String,
	status: String,
	detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSnapshot {
	logs_directory: String,
	manifests_directory: String,
	audit_entries: Vec<AuditEntry>,
	manifests: Vec<UploadManifest>,
}

#[tauri::command]
pub fn default_config() -> AppConfig {
	AppConfig::default()
}

#[tauri::command]
pub async fn load_config(app: AppHandle) -> Result<AppConfig, String> {
	run_blocking(move || load_config_inner(app)).await
}

fn load_config_inner(app: AppHandle) -> Result<AppConfig, String> {
	let path = config_path(&app)?;
	if !path.exists() {
		return Ok(AppConfig::default());
	}

	let text = fs::read_to_string(&path)
		.map_err(|error| format!("could not read config file: {error}"))?;
	let persisted: PersistedConfig =
		serde_json::from_str(&text).map_err(|error| format!("config file is invalid: {error}"))?;
	let mut config = AppConfig::default();
	config.storage = persisted.storage;
	apply_stored_secrets(&mut config).map_err(|error| error.to_string())?;
	Ok(config)
}

#[tauri::command]
pub async fn save_config(app: AppHandle, config: AppConfig) -> Result<(), String> {
	run_blocking(move || save_config_inner(app, config)).await
}

fn save_config_inner(app: AppHandle, config: AppConfig) -> Result<(), String> {
	let path = config_path(&app)?;
	if let Some(parent) = path.parent() {
		fs::create_dir_all(parent)
			.map_err(|error| format!("could not create config directory: {error}"))?;
	}
	persist_secrets(&config).map_err(|error| error.to_string())?;
	let persisted = PersistedConfig {
		storage: storage_without_secrets(config.storage),
	};
	let text = serde_json::to_string_pretty(&persisted)
		.map_err(|error| format!("could not serialize config: {error}"))?
		+ "\n";
	let tmp_path = path.with_extension("json.tmp");
	fs::write(&tmp_path, text).map_err(|error| format!("could not write config file: {error}"))?;
	fs::rename(tmp_path, path).map_err(|error| format!("could not save config file: {error}"))?;
	Ok(())
}

#[tauri::command]
pub async fn validate_package(
	config: AppConfig,
	zip_path: String,
	content_type: String,
) -> Result<ValidationReport, String> {
	run_blocking(move || {
		validate_zip(&config, zip_path, &content_type).map_err(|error| error.to_string())
	})
	.await
}

#[tauri::command]
pub async fn install_package(
	app: AppHandle,
	config: AppConfig,
	zip_path: String,
	content_type: String,
) -> Result<InstallReport, String> {
	run_blocking(move || {
		install_zip(&config, zip_path, &content_type, |event| {
			let _ = app.emit("install-progress", event);
		})
		.map_err(|error| error.to_string())
	})
	.await
}

#[tauri::command]
pub async fn list_uploads(config: AppConfig) -> Result<Vec<UploadManifest>, String> {
	run_blocking(move || list_manifests(&config).map_err(|error| error.to_string())).await
}

#[tauri::command]
pub async fn get_log_snapshot(config: AppConfig) -> Result<LogSnapshot, String> {
	run_blocking(move || log_snapshot(&config)).await
}

#[tauri::command]
pub async fn open_logs_folder(config: AppConfig) -> Result<(), String> {
	run_blocking(move || {
		let logs_directory = logs_directory(&config)?;
		fs::create_dir_all(&logs_directory)
			.map_err(|error| format!("could not create logs directory: {error}"))?;
		open_directory(logs_directory)
	})
	.await
}

#[tauri::command]
pub async fn rollback_upload(
	config: AppConfig,
	upload_id: String,
	force: bool,
) -> Result<RollbackReport, String> {
	run_blocking(move || {
		rollback_manifest(&config, &upload_id, force).map_err(|error| error.to_string())
	})
	.await
}

#[tauri::command]
pub async fn test_ftp_connection(config: FtpConfig) -> Result<(), String> {
	run_blocking(move || test_ftp(&config).map_err(|error| error.to_string())).await
}

#[tauri::command]
pub async fn test_sftp_connection(config: SftpConfig) -> Result<(), String> {
	run_blocking(move || test_sftp(&config).map_err(|error| error.to_string())).await
}

#[tauri::command]
pub async fn inspect_sftp_host(config: SftpConfig) -> Result<SftpHostFingerprint, String> {
	run_blocking(move || inspect_sftp_host_inner(&config).map_err(|error| error.to_string())).await
}

async fn run_blocking<T>(
	task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String>
where
	T: Send + 'static,
{
	async_runtime::spawn_blocking(task)
		.await
		.map_err(|error| error.to_string())?
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
	app.path()
		.app_config_dir()
		.map(|directory| directory.join(CONFIG_FILE))
		.map_err(|error| format!("could not resolve config directory: {error}"))
}

fn log_snapshot(config: &AppConfig) -> Result<LogSnapshot, String> {
	let logs_directory = logs_directory(config)?;
	let manifests_directory = manifests_directory(config)?;
	let audit_entries = read_audit_entries(&logs_directory)?;
	let manifests = list_manifests(config).map_err(|error| error.to_string())?;
	Ok(LogSnapshot {
		logs_directory: logs_directory.to_string_lossy().to_string(),
		manifests_directory: manifests_directory.to_string_lossy().to_string(),
		audit_entries,
		manifests,
	})
}

fn logs_directory(config: &AppConfig) -> Result<PathBuf, String> {
	if config.storage.server_root.as_os_str().is_empty() {
		return Err("Server root is required before opening logs".to_string());
	}
	Ok(config
		.storage
		.server_root
		.join(".fastdl-desktop")
		.join("logs"))
}

fn manifests_directory(config: &AppConfig) -> Result<PathBuf, String> {
	if config.storage.server_root.as_os_str().is_empty() {
		return Err("Server root is required before reading manifests".to_string());
	}
	Ok(config.storage.server_root.join(".uploads"))
}

fn read_audit_entries(logs_directory: &std::path::Path) -> Result<Vec<AuditEntry>, String> {
	let audit_path = logs_directory.join("audit.tsv");
	if !audit_path.exists() {
		return Ok(Vec::new());
	}
	let text = fs::read_to_string(&audit_path)
		.map_err(|error| format!("could not read audit log: {error}"))?;
	let mut entries = text
		.lines()
		.filter_map(parse_audit_line)
		.collect::<Vec<_>>();
	entries.reverse();
	entries.truncate(200);
	Ok(entries)
}

fn parse_audit_line(line: &str) -> Option<AuditEntry> {
	let mut parts = line.splitn(4, '\t');
	Some(AuditEntry {
		timestamp: parts.next()?.to_string(),
		action: parts.next()?.to_string(),
		status: parts.next()?.to_string(),
		detail: parts.next().unwrap_or("").to_string(),
	})
}

fn open_directory(directory: PathBuf) -> Result<(), String> {
	#[cfg(target_os = "windows")]
	{
		let mut command = Command::new("explorer");
		command.arg(directory);
		return spawn_directory(command);
	}

	#[cfg(target_os = "linux")]
	{
		let mut command = Command::new("xdg-open");
		command.arg(directory);
		return spawn_directory(command);
	}

	#[cfg(target_os = "macos")]
	{
		let mut command = Command::new("open");
		command.arg(directory);
		return spawn_directory(command);
	}

	#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
	{
		let _ = directory;
		Err("opening folders is not supported on this platform".to_string())
	}
}

fn spawn_directory(mut command: Command) -> Result<(), String> {
	command
		.spawn()
		.map(|_| ())
		.map_err(|error| format!("could not open logs directory: {error}"))
}

pub(crate) fn storage_without_secrets(mut storage: StorageConfig) -> StorageConfig {
	storage.server_ftp.password = None;
	storage.server_sftp.password = None;
	storage.server_sftp.private_key_passphrase = None;
	storage.ftp.password = None;
	storage.sftp.password = None;
	storage.sftp.private_key_passphrase = None;
	storage
}

fn apply_stored_secrets(config: &mut AppConfig) -> Result<(), crate::secrets::SecretError> {
	config.storage.server_ftp.password = read_secret(SERVER_FTP_PASSWORD_SECRET)?;
	config.storage.server_sftp.password = read_secret(SERVER_SFTP_PASSWORD_SECRET)?;
	config.storage.server_sftp.private_key_passphrase =
		read_secret(SERVER_SFTP_KEY_PASSPHRASE_SECRET)?;
	config.storage.ftp.password = read_secret(FTP_PASSWORD_SECRET)?;
	config.storage.sftp.password = read_secret(SFTP_PASSWORD_SECRET)?;
	config.storage.sftp.private_key_passphrase = read_secret(SFTP_KEY_PASSPHRASE_SECRET)?;
	Ok(())
}

fn persist_secrets(config: &AppConfig) -> Result<(), crate::secrets::SecretError> {
	write_secret(
		SERVER_FTP_PASSWORD_SECRET,
		config.storage.server_ftp.password.as_deref(),
	)?;
	write_secret(
		SERVER_SFTP_PASSWORD_SECRET,
		config.storage.server_sftp.password.as_deref(),
	)?;
	write_secret(
		SERVER_SFTP_KEY_PASSPHRASE_SECRET,
		config.storage.server_sftp.private_key_passphrase.as_deref(),
	)?;
	write_secret(FTP_PASSWORD_SECRET, config.storage.ftp.password.as_deref())?;
	write_secret(
		SFTP_PASSWORD_SECRET,
		config.storage.sftp.password.as_deref(),
	)?;
	write_secret(
		SFTP_KEY_PASSPHRASE_SECRET,
		config.storage.sftp.private_key_passphrase.as_deref(),
	)?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::storage_without_secrets;
	use crate::config::AppConfig;
	use std::path::PathBuf;

	#[test]
	fn storage_without_secrets_strips_remote_credentials() {
		let mut config = AppConfig::default();
		config.storage.server_ftp.password = Some("server-ftp-secret".to_string());
		config.storage.server_sftp.password = Some("server-sftp-secret".to_string());
		config.storage.server_sftp.private_key_passphrase = Some("server-key-secret".to_string());
		config.storage.ftp.password = Some("ftp-secret".to_string());
		config.storage.sftp.password = Some("sftp-secret".to_string());
		config.storage.sftp.private_key_passphrase = Some("key-secret".to_string());
		config.storage.sftp.private_key_path = Some(PathBuf::from("C:/keys/id_rsa"));
		config.storage.sftp.trusted_host_fingerprint = Some("SHA256:test".to_string());

		let storage = storage_without_secrets(config.storage);

		assert_eq!(storage.server_ftp.password, None);
		assert_eq!(storage.server_sftp.password, None);
		assert_eq!(storage.server_sftp.private_key_passphrase, None);
		assert_eq!(storage.ftp.password, None);
		assert_eq!(storage.sftp.password, None);
		assert_eq!(storage.sftp.private_key_passphrase, None);
		assert_eq!(
			storage.sftp.private_key_path,
			Some(PathBuf::from("C:/keys/id_rsa"))
		);
		assert_eq!(
			storage.sftp.trusted_host_fingerprint,
			Some("SHA256:test".to_string())
		);
	}
}
