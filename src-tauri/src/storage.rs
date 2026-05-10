use crate::audit::append_audit;
use crate::config::{AppConfig, CompressedFormat, FtpConfig, ServerInstallMode, SftpConfig};
use crate::ftp::delete_files as delete_ftp_files;
use crate::ftp::publish_files as publish_ftp_files;
use crate::sftp::delete_files as delete_sftp_files;
use crate::sftp::publish_files as publish_sftp_files;
use crate::validation::{normalize_zip_path, path_from_posix, validate_zip, ValidationReport};
use bzip2::write::BzEncoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;
use zip::ZipArchive;

const INTERNAL_ROOTS: &[&str] = &[".incoming", ".backups", ".uploads", ".fastdl-desktop"];

#[derive(Debug, Error)]
pub enum StorageError {
	#[error("{0}")]
	Validation(#[from] crate::validation::ValidationError),
	#[error("{0}")]
	Ftp(#[from] crate::ftp::FtpPublishError),
	#[error("{0}")]
	Sftp(#[from] crate::sftp::SftpError),
	#[error("server root is not configured")]
	MissingServerRoot,
	#[error("I/O operation failed: {0}")]
	Io(#[from] std::io::Error),
	#[error("invalid ZIP archive: {0}")]
	Zip(#[from] zip::result::ZipError),
	#[error("manifest JSON is invalid: {0}")]
	Json(#[from] serde_json::Error),
	#[error("{0}")]
	Rejected(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadManifest {
	pub upload_id: String,
	pub content_type: String,
	pub source_zip: String,
	pub status: String,
	pub started_at: String,
	pub completed_at: Option<String>,
	pub rolled_back_at: Option<String>,
	pub server_root: String,
	pub fastdl_root: Option<String>,
	pub installed_files: Vec<String>,
	pub installed_hashes: Vec<FileHash>,
	pub compressed_files: Vec<String>,
	pub compressed_hashes: Vec<FileHash>,
	pub backups: Vec<BackupEntry>,
	#[serde(default)]
	pub server_published_files: Vec<String>,
	#[serde(default)]
	pub server_rolled_back_files: Vec<String>,
	pub ftp_published_files: Vec<String>,
	pub sftp_published_files: Vec<String>,
	#[serde(default)]
	pub ftp_rolled_back_files: Vec<String>,
	#[serde(default)]
	pub sftp_rolled_back_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHash {
	pub path: String,
	pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
	pub target: String,
	pub backup: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
	pub upload_id: String,
	pub validation: ValidationReport,
	pub installed_files: Vec<String>,
	pub server_published_files: Vec<String>,
	pub compressed_files: Vec<String>,
	pub ftp_published_files: Vec<String>,
	pub sftp_published_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackReport {
	pub upload_id: String,
	pub deleted_files: Vec<String>,
	pub restored_files: Vec<String>,
	pub server_deleted_files: Vec<String>,
	pub ftp_deleted_files: Vec<String>,
	pub sftp_deleted_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
	pub step: String,
	pub message: String,
	pub current: usize,
	pub total: usize,
}

pub fn install_package(
	config: &AppConfig,
	zip_path: impl AsRef<Path>,
	content_type: &str,
) -> Result<InstallReport, StorageError> {
	install_package_with_progress(config, zip_path, content_type, |_| {})
}

pub fn install_package_with_progress(
	config: &AppConfig,
	zip_path: impl AsRef<Path>,
	content_type: &str,
	mut progress: impl FnMut(ProgressEvent),
) -> Result<InstallReport, StorageError> {
	emit_progress(&mut progress, "validating", "Validating ZIP package", 0, 0);
	let validation = validate_zip(config, zip_path.as_ref(), content_type)?;
	emit_progress(
		&mut progress,
		"validated",
		"ZIP package passed validation",
		validation.file_count,
		validation.file_count,
	);
	let roots = StorageRoots::new(config)?;
	let _lock = InstallLock::acquire(&roots)?;
	let upload_id = new_upload_id();
	let staging_dir = roots.incoming.join(&upload_id);
	fs::create_dir_all(&staging_dir)?;
	let mut manifest = UploadManifest {
		upload_id: upload_id.clone(),
		content_type: content_type.to_string(),
		source_zip: zip_path.as_ref().to_string_lossy().to_string(),
		status: "started".to_string(),
		started_at: now_rfc3339(),
		completed_at: None,
		rolled_back_at: None,
		server_root: roots.server.to_string_lossy().to_string(),
		fastdl_root: roots
			.fastdl
			.as_ref()
			.map(|path| path.to_string_lossy().to_string()),
		installed_files: Vec::new(),
		installed_hashes: Vec::new(),
		compressed_files: Vec::new(),
		compressed_hashes: Vec::new(),
		backups: Vec::new(),
		server_published_files: Vec::new(),
		server_rolled_back_files: Vec::new(),
		ftp_published_files: Vec::new(),
		sftp_published_files: Vec::new(),
		ftp_rolled_back_files: Vec::new(),
		sftp_rolled_back_files: Vec::new(),
	};
	write_manifest(&roots, &manifest)?;
	append_audit(&roots.server, "install", "started", &upload_id)?;
	emit_progress(&mut progress, "manifest", "Created upload manifest", 0, 0);

	let result = (|| {
		emit_progress(
			&mut progress,
			"extracting",
			"Extracting ZIP to staging",
			0,
			0,
		);
		extract_zip_to_staging(config, zip_path.as_ref(), &staging_dir, content_type)?;
		install_staged_files(config, &roots, &staging_dir, &mut manifest, &mut progress)?;
		manifest.server_published_files = publish_server_files(config, &roots, &manifest)?;
		write_manifest(&roots, &manifest)?;
		let published = publish_compressed_files(config, &roots, &manifest, &mut progress)?;
		manifest.ftp_published_files = published.ftp.clone();
		manifest.sftp_published_files = published.sftp.clone();
		manifest.status = "installed".to_string();
		manifest.completed_at = Some(now_rfc3339());
		write_manifest(&roots, &manifest)?;
		emit_progress(&mut progress, "complete", "Install complete", 1, 1);
		Ok::<RemotePublishResult, StorageError>(published)
	})();

	fs::remove_dir_all(&staging_dir).ok();
	match result {
		Ok(published) => {
			append_audit(&roots.server, "install", "installed", &upload_id)?;
			Ok(InstallReport {
				upload_id,
				validation,
				installed_files: manifest.installed_files,
				server_published_files: manifest.server_published_files,
				compressed_files: manifest.compressed_files,
				ftp_published_files: published.ftp,
				sftp_published_files: published.sftp,
			})
		}
		Err(error) => {
			emit_progress(
				&mut progress,
				"rollback",
				"Cleaning up failed install",
				0,
				0,
			);
			if cleanup_partial_install(&roots, &mut manifest).is_err() {
				manifest.status = "failed".to_string();
			}
			write_manifest(&roots, &manifest).ok();
			append_audit(
				&roots.server,
				"install",
				"failed",
				&format!("{}: {error}", manifest.upload_id),
			)
			.ok();
			Err(error)
		}
	}
}

fn cleanup_partial_install(
	roots: &StorageRoots,
	manifest: &mut UploadManifest,
) -> Result<(), StorageError> {
	for display_path in manifest
		.compressed_files
		.iter()
		.rev()
		.chain(manifest.installed_files.iter().rev())
	{
		let target = roots.resolve_display_path(display_path)?;
		if !target.exists() || target.is_dir() {
			continue;
		}
		if let Some(expected) = expected_hash(manifest, display_path) {
			if sha256_file(&target)? != expected {
				continue;
			}
		}
		fs::remove_file(target)?;
	}
	for backup in manifest.backups.iter().rev() {
		let target = roots.resolve_display_path(&backup.target)?;
		let backup_path = roots.resolve_display_path(&backup.backup)?;
		if !backup_path.exists() || target.exists() {
			continue;
		}
		if let Some(parent) = target.parent() {
			fs::create_dir_all(parent)?;
		}
		fs::rename(backup_path, target)?;
	}
	manifest.status = "rolled_back".to_string();
	manifest.rolled_back_at = Some(now_rfc3339());
	write_manifest(roots, manifest)?;
	Ok(())
}

pub fn list_uploads(config: &AppConfig) -> Result<Vec<UploadManifest>, StorageError> {
	let roots = StorageRoots::new(config)?;
	let mut manifests = Vec::new();
	if !roots.uploads.exists() {
		return Ok(manifests);
	}
	for entry in fs::read_dir(&roots.uploads)? {
		let path = entry?.path();
		if path.extension().and_then(|value| value.to_str()) == Some("json") {
			manifests.push(read_manifest_path(&path)?);
		}
	}
	manifests.sort_by(|left, right| right.started_at.cmp(&left.started_at));
	Ok(manifests)
}

pub fn rollback_upload(
	config: &AppConfig,
	upload_id: &str,
	force: bool,
) -> Result<RollbackReport, StorageError> {
	validate_upload_id(upload_id)?;
	let roots = StorageRoots::new(config)?;
	let _lock = InstallLock::acquire(&roots)?;
	let mut manifest = read_manifest(&roots, upload_id)?;
	if manifest.status == "rolled_back" {
		return Ok(RollbackReport {
			upload_id: upload_id.to_string(),
			deleted_files: Vec::new(),
			restored_files: Vec::new(),
			server_deleted_files: Vec::new(),
			ftp_deleted_files: Vec::new(),
			sftp_deleted_files: Vec::new(),
		});
	}
	if manifest.status == "installed" && !force {
		return Err(StorageError::Rejected(
			"refusing to roll back an installed upload without force".to_string(),
		));
	}

	let mut files_to_delete = Vec::new();
	for display_path in manifest
		.compressed_files
		.iter()
		.rev()
		.chain(manifest.installed_files.iter().rev())
	{
		let target = roots.resolve_display_path(display_path)?;
		if !target.exists() {
			continue;
		}
		if target.is_dir() {
			return Err(StorageError::Rejected(format!(
				"refusing to delete directory during rollback: {display_path}"
			)));
		}
		if let Some(expected) = expected_hash(&manifest, display_path) {
			let actual = sha256_file(&target)?;
			if actual != expected {
				return Err(StorageError::Rejected(format!(
					"refusing to delete modified file during rollback: {display_path}"
				)));
			}
		}
		files_to_delete.push((display_path.clone(), target));
	}

	let mut backups_to_restore = Vec::new();
	for backup in manifest.backups.iter().rev() {
		let target = roots.resolve_display_path(&backup.target)?;
		let backup_path = roots.resolve_display_path(&backup.backup)?;
		if !backup_path.exists() {
			continue;
		}
		if target.exists() {
			return Err(StorageError::Rejected(format!(
				"refusing to overwrite during rollback: {}",
				backup.target
			)));
		}
		backups_to_restore.push((backup.target.clone(), target, backup_path));
	}

	let server_deleted = rollback_server_files(config, &manifest)?;
	let remote_deleted = rollback_remote_files(config, &manifest)?;

	let mut deleted_files = Vec::new();
	for (display_path, target) in files_to_delete {
		fs::remove_file(&target)?;
		deleted_files.push(display_path);
	}

	let mut restored_files = Vec::new();
	for (display_path, target, backup_path) in backups_to_restore {
		if let Some(parent) = target.parent() {
			fs::create_dir_all(parent)?;
		}
		fs::rename(&backup_path, &target)?;
		restored_files.push(display_path);
	}

	manifest.server_rolled_back_files = server_deleted.clone();
	manifest.ftp_rolled_back_files = remote_deleted.ftp.clone();
	manifest.sftp_rolled_back_files = remote_deleted.sftp.clone();
	manifest.status = "rolled_back".to_string();
	manifest.rolled_back_at = Some(now_rfc3339());
	write_manifest(&roots, &manifest)?;
	append_audit(&roots.server, "rollback", "rolled_back", upload_id)?;
	Ok(RollbackReport {
		upload_id: upload_id.to_string(),
		deleted_files,
		restored_files,
		server_deleted_files: server_deleted,
		ftp_deleted_files: remote_deleted.ftp,
		sftp_deleted_files: remote_deleted.sftp,
	})
}

fn extract_zip_to_staging(
	config: &AppConfig,
	zip_path: &Path,
	staging_dir: &Path,
	content_type: &str,
) -> Result<(), StorageError> {
	let content_type = config
		.content_type(content_type)
		.ok_or_else(|| StorageError::Rejected(format!("unknown content type: {content_type}")))?;
	let file = File::open(zip_path)?;
	let mut archive = ZipArchive::new(file)?;
	for index in 0..archive.len() {
		let mut entry = archive.by_index(index)?;
		if entry.is_dir() {
			continue;
		}
		let normalized_path = normalize_zip_path(entry.name(), content_type.max_depth)?;
		let target = staging_dir.join(path_from_posix(&normalized_path));
		if !is_child_path(&target, staging_dir) {
			return Err(StorageError::Rejected(format!(
				"refusing to extract outside staging: {normalized_path}"
			)));
		}
		if let Some(parent) = target.parent() {
			fs::create_dir_all(parent)?;
		}
		let mut output = File::create(target)?;
		std::io::copy(&mut entry, &mut output)?;
	}
	Ok(())
}

fn install_staged_files(
	config: &AppConfig,
	roots: &StorageRoots,
	staging_dir: &Path,
	manifest: &mut UploadManifest,
	progress: &mut impl FnMut(ProgressEvent),
) -> Result<(), StorageError> {
	let mut staged_files = Vec::new();
	collect_files(staging_dir, &mut staged_files)?;
	staged_files.sort();
	emit_progress(
		progress,
		"planning",
		"Checking destination conflicts",
		0,
		staged_files.len(),
	);

	for source in &staged_files {
		let relative = source
			.strip_prefix(staging_dir)
			.map_err(|_| StorageError::Rejected("staged file escaped staging root".to_string()))?;
		validate_internal_root(relative)?;
		let target = roots.server.join(relative);
		if target.exists() && !config.storage.allow_overwrite {
			return Err(StorageError::Rejected(format!(
				"destination already exists: {}",
				display_path(relative)
			)));
		}
		for format in &config.storage.compressed_formats {
			let compressed_relative = compressed_relative_path(relative, *format);
			let compressed_target = roots.compressed_base().join(&compressed_relative);
			if compressed_target.exists() && !config.storage.allow_overwrite {
				return Err(StorageError::Rejected(format!(
					"destination already exists: {}",
					roots.display_path_for_compressed(&compressed_relative)
				)));
			}
		}
	}

	for (index, source) in staged_files.iter().enumerate() {
		let relative = source
			.strip_prefix(staging_dir)
			.map_err(|_| StorageError::Rejected("staged file escaped staging root".to_string()))?;
		emit_progress(
			progress,
			"installing",
			&format!("Installing {}", display_path(relative)),
			index + 1,
			staged_files.len(),
		);
		let target = roots.server.join(relative);
		if let Some(parent) = target.parent() {
			fs::create_dir_all(parent)?;
		}
		backup_existing(config, roots, &target, relative, manifest)?;
		fs::rename(source, &target)?;
		let display = display_path(relative);
		manifest.installed_files.push(display.clone());
		manifest.installed_hashes.push(FileHash {
			path: display,
			sha256: sha256_file(&target)?,
		});
		write_manifest(roots, manifest)?;

		for (format_index, format) in config.storage.compressed_formats.iter().enumerate() {
			let compressed_relative = compressed_relative_path(relative, *format);
			let compressed_target = roots.compressed_base().join(&compressed_relative);
			if let Some(parent) = compressed_target.parent() {
				fs::create_dir_all(parent)?;
			}
			backup_existing(
				config,
				roots,
				&compressed_target,
				&compressed_relative,
				manifest,
			)?;
			emit_progress(
				progress,
				"compressing",
				&format!(
					"Compressing {}",
					roots.display_path_for_compressed(&compressed_relative)
				),
				format_index + 1,
				config.storage.compressed_formats.len(),
			);
			write_compressed_copy(&target, &compressed_target, *format)?;
			let display = roots.display_path_for_compressed(&compressed_relative);
			manifest.compressed_files.push(display.clone());
			manifest.compressed_hashes.push(FileHash {
				path: display,
				sha256: sha256_file(&compressed_target)?,
			});
			write_manifest(roots, manifest)?;
		}
	}
	Ok(())
}

fn backup_existing(
	config: &AppConfig,
	roots: &StorageRoots,
	target: &Path,
	relative: &Path,
	manifest: &mut UploadManifest,
) -> Result<(), StorageError> {
	if !target.exists() {
		return Ok(());
	}
	if !config.storage.backup_existing {
		fs::remove_file(target)?;
		return Ok(());
	}
	let backup_path = roots.backups.join(&manifest.upload_id).join(relative);
	if let Some(parent) = backup_path.parent() {
		fs::create_dir_all(parent)?;
	}
	fs::rename(target, &backup_path)?;
	manifest.backups.push(BackupEntry {
		target: roots.display_path_for_target(target)?,
		backup: roots.display_path_for_target(&backup_path)?,
	});
	write_manifest(roots, manifest)?;
	Ok(())
}

#[derive(Debug, Clone)]
struct RemotePublishResult {
	ftp: Vec<String>,
	sftp: Vec<String>,
}

fn publish_server_files(
	config: &AppConfig,
	roots: &StorageRoots,
	manifest: &UploadManifest,
) -> Result<Vec<String>, StorageError> {
	let files = manifest
		.installed_files
		.iter()
		.map(|display| {
			let local_path = roots.resolve_display_path(display)?;
			Ok((local_path, display.clone()))
		})
		.collect::<Result<Vec<_>, StorageError>>()?;

	match config.storage.server_install_mode {
		ServerInstallMode::Local => Ok(Vec::new()),
		ServerInstallMode::Ftp => {
			let server_ftp = enabled_ftp_config(&config.storage.server_ftp);
			publish_ftp_files(&server_ftp, &files)?;
			Ok(files.into_iter().map(|(_, relative)| relative).collect())
		}
		ServerInstallMode::Sftp => {
			let server_sftp = enabled_sftp_config(&config.storage.server_sftp);
			publish_sftp_files(&server_sftp, &files)?;
			Ok(files.into_iter().map(|(_, relative)| relative).collect())
		}
	}
}

fn rollback_server_files(
	config: &AppConfig,
	manifest: &UploadManifest,
) -> Result<Vec<String>, StorageError> {
	if manifest.server_published_files.is_empty() {
		return Ok(Vec::new());
	}

	match config.storage.server_install_mode {
		ServerInstallMode::Local => Err(StorageError::Rejected(
			"rollback needs the original FTP/SFTP game server mode to remove previously published server files".to_string(),
		)),
		ServerInstallMode::Ftp => {
			let server_ftp = enabled_ftp_config(&config.storage.server_ftp);
			Ok(delete_ftp_files(&server_ftp, &manifest.server_published_files)?)
		}
		ServerInstallMode::Sftp => {
			let server_sftp = enabled_sftp_config(&config.storage.server_sftp);
			Ok(delete_sftp_files(
				&server_sftp,
				&manifest.server_published_files,
			)?)
		}
	}
}

fn enabled_ftp_config(config: &FtpConfig) -> FtpConfig {
	let mut config = config.clone();
	config.enabled = true;
	config
}

fn enabled_sftp_config(config: &SftpConfig) -> SftpConfig {
	let mut config = config.clone();
	config.enabled = true;
	config
}

fn rollback_remote_files(
	config: &AppConfig,
	manifest: &UploadManifest,
) -> Result<RemotePublishResult, StorageError> {
	if !manifest.ftp_published_files.is_empty() && !config.storage.ftp.enabled {
		return Err(StorageError::Rejected(
			"rollback needs FTP enabled to remove previously published FTP files".to_string(),
		));
	}
	if !manifest.sftp_published_files.is_empty() && !config.storage.sftp.enabled {
		return Err(StorageError::Rejected(
			"rollback needs SFTP enabled to remove previously published SFTP files".to_string(),
		));
	}
	let ftp = delete_ftp_files(&config.storage.ftp, &manifest.ftp_published_files)?;
	let sftp = delete_sftp_files(&config.storage.sftp, &manifest.sftp_published_files)?;
	Ok(RemotePublishResult { ftp, sftp })
}

fn publish_compressed_files(
	config: &AppConfig,
	roots: &StorageRoots,
	manifest: &UploadManifest,
	progress: &mut impl FnMut(ProgressEvent),
) -> Result<RemotePublishResult, StorageError> {
	let files = manifest
		.compressed_files
		.iter()
		.map(|display| {
			let local_path = roots.resolve_display_path(display)?;
			let relative = display
				.strip_prefix("fastdl/")
				.unwrap_or(display)
				.to_string();
			Ok((local_path, relative))
		})
		.collect::<Result<Vec<_>, StorageError>>()?;
	let relative_files = files
		.iter()
		.map(|(_, relative)| relative.clone())
		.collect::<Vec<_>>();
	if config.storage.ftp.enabled {
		emit_progress(
			progress,
			"uploading-ftp",
			"Publishing FastDL files over FTP",
			0,
			files.len(),
		);
	}
	publish_ftp_files(&config.storage.ftp, &files)?;
	if config.storage.sftp.enabled {
		emit_progress(
			progress,
			"uploading-sftp",
			"Publishing FastDL files over SFTP",
			0,
			files.len(),
		);
	}
	publish_sftp_files(&config.storage.sftp, &files)?;
	Ok(RemotePublishResult {
		ftp: if config.storage.ftp.enabled {
			relative_files.clone()
		} else {
			Vec::new()
		},
		sftp: if config.storage.sftp.enabled {
			relative_files
		} else {
			Vec::new()
		},
	})
}

fn emit_progress(
	progress: &mut impl FnMut(ProgressEvent),
	step: &str,
	message: &str,
	current: usize,
	total: usize,
) {
	progress(ProgressEvent {
		step: step.to_string(),
		message: message.to_string(),
		current,
		total,
	});
}

fn write_compressed_copy(
	source: &Path,
	target: &Path,
	format: CompressedFormat,
) -> Result<(), StorageError> {
	let mut input = File::open(source)?;
	let output = File::create(target)?;
	match format {
		CompressedFormat::Gz => {
			let mut encoder = GzEncoder::new(output, flate2::Compression::default());
			std::io::copy(&mut input, &mut encoder)?;
			encoder.finish()?;
		}
		CompressedFormat::Bz2 => {
			let mut encoder = BzEncoder::new(output, bzip2::Compression::default());
			std::io::copy(&mut input, &mut encoder)?;
			encoder.finish()?;
		}
	}
	Ok(())
}

fn collect_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), StorageError> {
	for entry in fs::read_dir(root)? {
		let path = entry?.path();
		if path.is_dir() {
			collect_files(&path, files)?;
		} else if path.is_file() {
			files.push(path);
		}
	}
	Ok(())
}

fn validate_internal_root(relative: &Path) -> Result<(), StorageError> {
	if let Some(first) = relative
		.components()
		.next()
		.and_then(|component| component.as_os_str().to_str())
	{
		if INTERNAL_ROOTS
			.iter()
			.any(|internal| first.eq_ignore_ascii_case(internal))
		{
			return Err(StorageError::Rejected(format!(
				"refusing to install into internal directory: {first}"
			)));
		}
	}
	Ok(())
}

fn compressed_relative_path(relative: &Path, format: CompressedFormat) -> PathBuf {
	let extension = match format {
		CompressedFormat::Gz => "gz",
		CompressedFormat::Bz2 => "bz2",
	};
	PathBuf::from(format!("{}.{}", display_path(relative), extension))
}

fn display_path(relative: &Path) -> String {
	relative
		.components()
		.map(|component| component.as_os_str().to_string_lossy().to_string())
		.collect::<Vec<_>>()
		.join("/")
}

fn sha256_file(path: &Path) -> Result<String, StorageError> {
	let mut handle = File::open(path)?;
	let mut digest = Sha256::new();
	let mut buffer = [0_u8; 1024 * 64];
	loop {
		let read = handle.read(&mut buffer)?;
		if read == 0 {
			break;
		}
		digest.update(&buffer[..read]);
	}
	Ok(format!("{:x}", digest.finalize()))
}

fn expected_hash(manifest: &UploadManifest, path: &str) -> Option<String> {
	manifest
		.installed_hashes
		.iter()
		.chain(manifest.compressed_hashes.iter())
		.find(|entry| entry.path == path)
		.map(|entry| entry.sha256.clone())
}

fn write_manifest(roots: &StorageRoots, manifest: &UploadManifest) -> Result<(), StorageError> {
	fs::create_dir_all(&roots.uploads)?;
	let path = roots.manifest_path(&manifest.upload_id);
	let tmp_path = path.with_extension("json.tmp");
	fs::write(&tmp_path, serde_json::to_string_pretty(manifest)? + "\n")?;
	fs::rename(tmp_path, path)?;
	Ok(())
}

fn read_manifest(roots: &StorageRoots, upload_id: &str) -> Result<UploadManifest, StorageError> {
	read_manifest_path(&roots.manifest_path(upload_id))
}

fn read_manifest_path(path: &Path) -> Result<UploadManifest, StorageError> {
	let text = fs::read_to_string(path)?;
	Ok(serde_json::from_str(&text)?)
}

fn validate_upload_id(upload_id: &str) -> Result<(), StorageError> {
	if upload_id.is_empty()
		|| upload_id.contains('/')
		|| upload_id.contains('\\')
		|| upload_id.contains("..")
		|| upload_id.ends_with(".json")
	{
		return Err(StorageError::Rejected("invalid upload id".to_string()));
	}
	Ok(())
}

fn new_upload_id() -> String {
	format!(
		"{}-{}",
		OffsetDateTime::now_utc().unix_timestamp(),
		Uuid::new_v4().simple()
	)
}

fn now_rfc3339() -> String {
	OffsetDateTime::now_utc()
		.format(&time::format_description::well_known::Rfc3339)
		.unwrap_or_else(|_| "unknown-time".to_string())
}

fn is_child_path(path: &Path, parent: &Path) -> bool {
	let Ok(path) = path.canonicalize() else {
		return path.starts_with(parent);
	};
	let Ok(parent) = parent.canonicalize() else {
		return false;
	};
	path == parent || path.starts_with(parent)
}

struct StorageRoots {
	server: PathBuf,
	fastdl: Option<PathBuf>,
	incoming: PathBuf,
	backups: PathBuf,
	uploads: PathBuf,
	lock_path: PathBuf,
}

impl StorageRoots {
	fn new(config: &AppConfig) -> Result<Self, StorageError> {
		if config.storage.server_root.as_os_str().is_empty() {
			return Err(StorageError::MissingServerRoot);
		}
		let server = config.storage.server_root.clone();
		fs::create_dir_all(&server)?;
		let fastdl = config.storage.fastdl_root.clone();
		if let Some(fastdl) = &fastdl {
			fs::create_dir_all(fastdl)?;
		}
		let incoming = server.join(".incoming");
		let backups = server.join(".backups");
		let uploads = server.join(".uploads");
		fs::create_dir_all(&incoming)?;
		fs::create_dir_all(&backups)?;
		fs::create_dir_all(&uploads)?;
		Ok(Self {
			lock_path: server.join(".fastdl-upload.lock"),
			server,
			fastdl,
			incoming,
			backups,
			uploads,
		})
	}

	fn compressed_base(&self) -> &Path {
		self.fastdl.as_deref().unwrap_or(&self.server)
	}

	fn manifest_path(&self, upload_id: &str) -> PathBuf {
		self.uploads.join(format!("{upload_id}.json"))
	}

	fn display_path_for_compressed(&self, relative: &Path) -> String {
		if self.fastdl.is_some() {
			format!("fastdl/{}", display_path(relative))
		} else {
			display_path(relative)
		}
	}

	fn display_path_for_target(&self, target: &Path) -> Result<String, StorageError> {
		if let Ok(relative) = target.strip_prefix(&self.server) {
			return Ok(display_path(relative));
		}
		if let Some(fastdl) = &self.fastdl {
			if let Ok(relative) = target.strip_prefix(fastdl) {
				return Ok(format!("fastdl/{}", display_path(relative)));
			}
		}
		Err(StorageError::Rejected(
			"path is outside configured roots".to_string(),
		))
	}

	fn resolve_display_path(&self, display: &str) -> Result<PathBuf, StorageError> {
		let (base, relative) = if let Some(relative) = display.strip_prefix("fastdl/") {
			let fastdl = self.fastdl.as_ref().ok_or_else(|| {
				StorageError::Rejected(format!(
					"manifest references FastDL root but none is configured: {display}"
				))
			})?;
			(fastdl, relative)
		} else {
			(&self.server, display)
		};
		if relative.contains('\\') || relative.contains("..") || relative.starts_with('/') {
			return Err(StorageError::Rejected(format!(
				"unsafe manifest path: {display}"
			)));
		}
		Ok(base.join(path_from_posix(relative)))
	}
}

struct InstallLock {
	path: PathBuf,
}

impl InstallLock {
	fn acquire(roots: &StorageRoots) -> Result<Self, StorageError> {
		let mut handle = OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&roots.lock_path)
			.map_err(|error| {
				if error.kind() == std::io::ErrorKind::AlreadyExists {
					StorageError::Rejected(
						"another install or rollback is already running".to_string(),
					)
				} else {
					StorageError::Io(error)
				}
			})?;
		writeln!(handle, "pid={}", std::process::id())?;
		Ok(Self {
			path: roots.lock_path.clone(),
		})
	}
}

impl Drop for InstallLock {
	fn drop(&mut self) {
		fs::remove_file(&self.path).ok();
	}
}
