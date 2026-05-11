use crate::config::{AppConfig, CompressedFormat, ContentTypeConfig};
use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;
use zip::ZipArchive;

const WINDOWS_RESERVED_NAMES: &[&str] = &[
	"con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
	"com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

#[derive(Debug, Error)]
pub enum ValidationError {
	#[error("unknown content type: {0}")]
	UnknownContentType(String),
	#[error("select the server root directory before validating")]
	MissingServerRoot,
	#[error("could not open the package archive: {0}")]
	OpenPackage(#[source] std::io::Error),
	#[error("file is not a valid ZIP archive: {0}")]
	InvalidZip(#[source] zip::result::ZipError),
	#[error("file is not a valid TAR archive: {0}")]
	InvalidTar(#[source] std::io::Error),
	#[error("{0}")]
	Rejected(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
	pub content_type: String,
	pub file_count: usize,
	pub total_uncompressed_bytes: u64,
	pub folders: Vec<CountItem>,
	pub extensions: Vec<CountItem>,
	pub largest_files: Vec<FileSummary>,
	pub files: Vec<FileSummary>,
	pub compressed_files: Vec<String>,
	pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountItem {
	pub name: String,
	pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSummary {
	pub path: String,
	pub size: u64,
	pub compressed_size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchiveKind {
	Zip,
	Tar,
	TarGz,
	TarBz2,
}

pub fn validate_zip(
	config: &AppConfig,
	package_path: impl AsRef<Path>,
	content_type_id: &str,
) -> Result<ValidationReport, ValidationError> {
	let content_type = config
		.content_type(content_type_id)
		.ok_or_else(|| ValidationError::UnknownContentType(content_type_id.to_string()))?;
	if config.storage.server_root.as_os_str().is_empty() {
		return Err(ValidationError::MissingServerRoot);
	}

	let package_path = package_path.as_ref();
	let kind = archive_kind(package_path)?;
	let mut files = Vec::new();
	let mut package_paths = BTreeSet::new();
	let mut res_files = Vec::new();
	let mut seen_paths = BTreeSet::new();
	let mut found_extensions = BTreeSet::new();
	let mut total_compressed_bytes = 0_u64;
	let mut total_uncompressed_bytes = 0_u64;

	let mut process_entry = |raw_name: String,
	                         size: u64,
	                         compressed_size: u64,
	                         is_symlink: bool,
	                         reader: &mut dyn Read|
	 -> Result<(), ValidationError> {
		let normalized_path = normalize_zip_path(&raw_name, content_type.max_depth)?;
		validate_entry_path(&normalized_path, content_type)?;
		if is_symlink {
			return Err(ValidationError::Rejected(format!(
				"{raw_name}: symlinks are not accepted"
			)));
		}

		let folded = normalized_path.to_ascii_lowercase();
		if !seen_paths.insert(folded) {
			return Err(ValidationError::Rejected(format!(
				"{normalized_path}: duplicate file or case collision"
			)));
		}
		package_paths.insert(normalized_path.to_ascii_lowercase());

		let extension = extension_of(&normalized_path)?;
		if !content_type
			.allowed_extensions
			.iter()
			.any(|allowed| allowed == &extension)
		{
			return Err(ValidationError::Rejected(format!(
				"{normalized_path}: extension {extension} is not allowed for {}",
				content_type.name
			)));
		}
		if !matches_path_rule(&normalized_path, &extension, content_type) {
			return Err(ValidationError::Rejected(format!(
				"{normalized_path}: folder/extension is not allowed for {}",
				content_type.name
			)));
		}
		if size > content_type.max_file_bytes {
			return Err(ValidationError::Rejected(format!(
				"{normalized_path}: file exceeds the per-file size limit"
			)));
		}

		total_compressed_bytes = total_compressed_bytes.saturating_add(compressed_size);
		if total_compressed_bytes > content_type.max_compressed_bytes {
			return Err(ValidationError::Rejected(
				"compressed content exceeds the configured limit".to_string(),
			));
		}
		total_uncompressed_bytes = total_uncompressed_bytes.saturating_add(size);
		if total_uncompressed_bytes > content_type.max_uncompressed_bytes {
			return Err(ValidationError::Rejected(
				"uncompressed content exceeds the configured limit".to_string(),
			));
		}

		if is_map_res_file(&normalized_path) {
			let mut content = String::new();
			reader.read_to_string(&mut content).map_err(|error| {
				ValidationError::Rejected(format!(
					"{normalized_path}: could not read .res file: {error}"
				))
			})?;
			res_files.push((normalized_path.clone(), content));
		}

		found_extensions.insert(extension);
		files.push(FileSummary {
			path: normalized_path,
			size,
			compressed_size,
		});
		Ok(())
	};

	match kind {
		ArchiveKind::Zip => {
			let file = File::open(package_path).map_err(ValidationError::OpenPackage)?;
			let mut archive = ZipArchive::new(file).map_err(ValidationError::InvalidZip)?;
			for index in 0..archive.len() {
				let mut entry = archive
					.by_index(index)
					.map_err(ValidationError::InvalidZip)?;
				if entry.is_dir() {
					continue;
				}
				process_entry(
					entry.name().to_string(),
					entry.size(),
					entry.compressed_size(),
					is_zip_symlink(entry.unix_mode()),
					&mut entry,
				)?;
			}
		}
		ArchiveKind::Tar | ArchiveKind::TarGz | ArchiveKind::TarBz2 => {
			let package_compressed_bytes = File::open(package_path)
				.and_then(|file| file.metadata())
				.map_err(ValidationError::OpenPackage)?
				.len();
			if package_compressed_bytes > content_type.max_compressed_bytes {
				return Err(ValidationError::Rejected(
					"compressed content exceeds the configured limit".to_string(),
				));
			}
			match kind {
				ArchiveKind::Tar => {
					let file = File::open(package_path).map_err(ValidationError::OpenPackage)?;
					validate_tar_archive(tar::Archive::new(file), &mut process_entry)?;
				}
				ArchiveKind::TarGz => {
					let file = File::open(package_path).map_err(ValidationError::OpenPackage)?;
					validate_tar_archive(
						tar::Archive::new(GzDecoder::new(file)),
						&mut process_entry,
					)?;
				}
				ArchiveKind::TarBz2 => {
					let file = File::open(package_path).map_err(ValidationError::OpenPackage)?;
					validate_tar_archive(
						tar::Archive::new(BzDecoder::new(file)),
						&mut process_entry,
					)?;
				}
				ArchiveKind::Zip => unreachable!("ZIP entries are handled separately"),
			}
		}
	}

	if files.is_empty() {
		return Err(ValidationError::Rejected("archive is empty".to_string()));
	}
	if files.len() > content_type.max_file_count {
		return Err(ValidationError::Rejected(
			"archive exceeds the file count limit".to_string(),
		));
	}
	for required in &content_type.required_extensions {
		if !found_extensions.contains(required) {
			return Err(ValidationError::Rejected(format!(
				"archive must contain the required extension {required}"
			)));
		}
	}

	validate_res_files(config, content_type, &res_files, &package_paths)?;
	if !content_type.required_any_extensions.is_empty()
		&& !content_type
			.required_any_extensions
			.iter()
			.any(|required| found_extensions.contains(required))
	{
		return Err(ValidationError::Rejected(format!(
			"archive must contain at least one of these extensions: {}",
			content_type.required_any_extensions.join(", ")
		)));
	}

	let compressed_files = planned_compressed_files(config, &files);
	let conflicts = destination_conflicts(config, &files, &compressed_files);
	let folders = count_by(
		files
			.iter()
			.map(|file| file.path.split('/').next().unwrap_or("").to_string()),
	);
	let extensions = count_by(
		files
			.iter()
			.map(|file| extension_of(&file.path).unwrap_or_else(|_| "(no extension)".to_string())),
	);
	let mut largest_files = files.clone();
	largest_files.sort_by(|left, right| right.size.cmp(&left.size));
	largest_files.truncate(5);

	Ok(ValidationReport {
		content_type: content_type.name.clone(),
		file_count: files.len(),
		total_uncompressed_bytes,
		folders,
		extensions,
		largest_files,
		files,
		compressed_files,
		conflicts,
	})
}

pub(crate) fn archive_kind(path: &Path) -> Result<ArchiveKind, ValidationError> {
	let file_name = path
		.file_name()
		.and_then(|value| value.to_str())
		.unwrap_or("")
		.to_ascii_lowercase();
	if file_name.ends_with(".zip") {
		return Ok(ArchiveKind::Zip);
	}
	if file_name.ends_with(".tar") {
		return Ok(ArchiveKind::Tar);
	}
	if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") {
		return Ok(ArchiveKind::TarGz);
	}
	if file_name.ends_with(".tar.bz2") || file_name.ends_with(".tbz2") {
		return Ok(ArchiveKind::TarBz2);
	}
	Err(ValidationError::Rejected(
		"unsupported archive format; use .zip, .tar, .tar.gz, .tgz, .tar.bz2, or .tbz2".to_string(),
	))
}

fn validate_tar_archive<R>(
	mut archive: tar::Archive<R>,
	process_entry: &mut impl FnMut(String, u64, u64, bool, &mut dyn Read) -> Result<(), ValidationError>,
) -> Result<(), ValidationError>
where
	R: Read,
{
	for entry in archive.entries().map_err(ValidationError::InvalidTar)? {
		let mut entry = entry.map_err(ValidationError::InvalidTar)?;
		let entry_type = entry.header().entry_type();
		if entry_type.is_dir() {
			continue;
		}
		let raw_name = entry
			.path()
			.map_err(ValidationError::InvalidTar)?
			.to_string_lossy()
			.to_string();
		let size = entry.size();
		process_entry(
			raw_name,
			size,
			0,
			entry_type.is_symlink() || entry_type.is_hard_link(),
			&mut entry,
		)?;
	}
	Ok(())
}

pub(crate) fn normalize_zip_path(
	raw_name: &str,
	max_depth: usize,
) -> Result<String, ValidationError> {
	if raw_name.is_empty() || raw_name.contains('\0') {
		return Err(ValidationError::Rejected(
			"entry has an empty or invalid name".to_string(),
		));
	}
	if raw_name.contains('\\') {
		return Err(ValidationError::Rejected(format!(
			"{raw_name}: use / as the folder separator"
		)));
	}
	if raw_name.starts_with('/') || raw_name.starts_with("//") {
		return Err(ValidationError::Rejected(format!(
			"{raw_name}: absolute paths are not accepted"
		)));
	}
	if raw_name
		.split('/')
		.next()
		.is_some_and(|first| first.contains(':'))
	{
		return Err(ValidationError::Rejected(format!(
			"{raw_name}: drive paths are not accepted"
		)));
	}

	let path = Path::new(raw_name);
	let mut parts = Vec::new();
	for component in path.components() {
		let Component::Normal(part) = component else {
			return Err(ValidationError::Rejected(format!(
				"{raw_name}: path traversal is not accepted"
			)));
		};
		let part = part.to_string_lossy().to_string();
		if part.is_empty() || part == "." || part == ".." {
			return Err(ValidationError::Rejected(format!(
				"{raw_name}: path traversal is not accepted"
			)));
		}
		if part.contains(':') {
			return Err(ValidationError::Rejected(format!(
				"{raw_name}: names containing : are not accepted"
			)));
		}
		if part.ends_with(' ') || part.ends_with('.') {
			return Err(ValidationError::Rejected(format!(
				"{raw_name}: names ending with space/dot are not accepted"
			)));
		}
		let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
		if WINDOWS_RESERVED_NAMES.contains(&stem.as_str()) {
			return Err(ValidationError::Rejected(format!(
				"{raw_name}: Windows reserved names are not accepted"
			)));
		}
		parts.push(part);
	}
	if parts.is_empty() {
		return Err(ValidationError::Rejected(
			"entry has an empty path".to_string(),
		));
	}
	if parts.len() > max_depth {
		return Err(ValidationError::Rejected(format!(
			"{raw_name}: folder depth exceeds the limit"
		)));
	}

	Ok(parts.join("/"))
}

fn validate_entry_path(
	path: &str,
	content_type: &ContentTypeConfig,
) -> Result<(), ValidationError> {
	if content_type.require_lowercase_paths && path != path.to_ascii_lowercase() {
		return Err(ValidationError::Rejected(format!(
			"{path}: path must be lowercase"
		)));
	}
	Ok(())
}

fn validate_res_files(
	config: &AppConfig,
	content_type: &ContentTypeConfig,
	res_files: &[(String, String)],
	package_paths: &BTreeSet<String>,
) -> Result<(), ValidationError> {
	for (res_path, content) in res_files {
		for (line_number, line) in content.lines().enumerate() {
			let resource = clean_res_line(line);
			if resource.is_empty() {
				continue;
			}
			let normalized =
				normalize_zip_path(&resource, content_type.max_depth).map_err(|error| {
					ValidationError::Rejected(format!(
						"{res_path}:{}: invalid resource path: {error}",
						line_number + 1
					))
				})?;
			if package_paths.contains(&normalized.to_ascii_lowercase()) {
				continue;
			}
			let target = config
				.storage
				.server_root
				.join(path_from_posix(&normalized));
			if target.exists() {
				continue;
			}
			return Err(ValidationError::Rejected(format!(
				"{res_path}:{}: listed resource is missing from the ZIP and server root: {normalized}",
				line_number + 1
			)));
		}
	}
	Ok(())
}

fn clean_res_line(line: &str) -> String {
	let mut value = line.trim();
	if value.is_empty()
		|| value.starts_with("//")
		|| value.starts_with('#')
		|| value.starts_with(';')
	{
		return String::new();
	}
	if let Some((before_comment, _)) = value.split_once("//") {
		value = before_comment.trim();
	}
	value.trim_matches('"').trim().to_string()
}

fn is_map_res_file(path: &str) -> bool {
	let mut parts = path.split('/');
	matches!(
		(parts.next(), parts.next(), parts.next()),
		(Some("maps"), Some(file), None) if file.to_ascii_lowercase().ends_with(".res")
	)
}

fn extension_of(path: &str) -> Result<String, ValidationError> {
	let extension = Path::new(path)
		.extension()
		.and_then(|extension| extension.to_str())
		.map(|extension| format!(".{}", extension.to_ascii_lowercase()));
	extension.ok_or_else(|| ValidationError::Rejected(format!("{path}: file has no extension")))
}

fn matches_path_rule(path: &str, extension: &str, content_type: &ContentTypeConfig) -> bool {
	if content_type.path_rules.is_empty() {
		return true;
	}
	content_type.path_rules.iter().any(|rule| {
		rule.extensions.iter().any(|allowed| allowed == extension)
			&& (rule.prefix.is_empty()
				|| path == rule.prefix
				|| path.starts_with(&format!("{}/", rule.prefix)))
	})
}

fn is_zip_symlink(unix_mode: Option<u32>) -> bool {
	unix_mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

fn planned_compressed_files(config: &AppConfig, files: &[FileSummary]) -> Vec<String> {
	files
		.iter()
		.flat_map(|file| {
			config.storage.compressed_formats.iter().map(move |format| {
				let extension = match format {
					CompressedFormat::Gz => "gz",
					CompressedFormat::Bz2 => "bz2",
				};
				if config.storage.fastdl_root.is_some() {
					format!("fastdl/{}.{}", file.path, extension)
				} else {
					format!("{}.{}", file.path, extension)
				}
			})
		})
		.collect()
}

fn destination_conflicts(
	config: &AppConfig,
	files: &[FileSummary],
	compressed_files: &[String],
) -> Vec<String> {
	let mut conflicts = Vec::new();
	for file in files {
		let target = config.storage.server_root.join(path_from_posix(&file.path));
		if target.exists() {
			conflicts.push(file.path.clone());
		}
	}
	let compressed_base = config
		.storage
		.fastdl_root
		.as_ref()
		.unwrap_or(&config.storage.server_root);
	for compressed_file in compressed_files {
		let display_path = compressed_file
			.strip_prefix("fastdl/")
			.unwrap_or(compressed_file);
		if compressed_base.join(path_from_posix(display_path)).exists() {
			conflicts.push(compressed_file.clone());
		}
	}
	conflicts
}

pub(crate) fn path_from_posix(path: &str) -> PathBuf {
	path.split('/').collect()
}

fn count_by(values: impl Iterator<Item = String>) -> Vec<CountItem> {
	let mut counts = BTreeMap::<String, usize>::new();
	for value in values {
		*counts.entry(value).or_insert(0) += 1;
	}
	counts
		.into_iter()
		.map(|(name, count)| CountItem { name, count })
		.collect()
}
