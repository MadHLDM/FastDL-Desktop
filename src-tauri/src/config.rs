use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
	pub storage: StorageConfig,
	pub content_types: Vec<ContentTypeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageConfig {
	pub server_root: PathBuf,
	#[serde(default)]
	pub server_install_mode: ServerInstallMode,
	#[serde(default)]
	pub server_ftp: FtpConfig,
	#[serde(default)]
	pub server_sftp: SftpConfig,
	pub fastdl_root: Option<PathBuf>,
	pub compressed_formats: Vec<CompressedFormat>,
	pub allow_overwrite: bool,
	pub backup_existing: bool,
	pub ftp: FtpConfig,
	pub sftp: SftpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FtpConfig {
	pub enabled: bool,
	pub host: Option<String>,
	pub port: u16,
	pub username: Option<String>,
	pub password: Option<String>,
	pub remote_fastdl_root: Option<String>,
}

impl Default for FtpConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			host: None,
			port: 21,
			username: None,
			password: None,
			remote_fastdl_root: None,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpConfig {
	pub enabled: bool,
	pub host: Option<String>,
	pub port: u16,
	pub username: Option<String>,
	pub password: Option<String>,
	pub private_key_path: Option<PathBuf>,
	pub private_key_passphrase: Option<String>,
	pub remote_fastdl_root: Option<String>,
	pub trusted_host_fingerprint: Option<String>,
}

impl Default for SftpConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			host: None,
			port: 22,
			username: None,
			password: None,
			private_key_path: None,
			private_key_passphrase: None,
			remote_fastdl_root: None,
			trusted_host_fingerprint: None,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ServerInstallMode {
	#[default]
	Local,
	Ftp,
	Sftp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentTypeConfig {
	pub id: String,
	pub name: String,
	pub max_compressed_bytes: u64,
	pub max_uncompressed_bytes: u64,
	pub max_file_bytes: u64,
	pub max_file_count: usize,
	pub max_depth: usize,
	pub require_lowercase_paths: bool,
	pub required_extensions: Vec<String>,
	pub required_any_extensions: Vec<String>,
	pub allowed_extensions: Vec<String>,
	pub path_rules: Vec<PathRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathRule {
	pub prefix: String,
	pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressedFormat {
	Gz,
	Bz2,
}

impl AppConfig {
	pub fn content_type(&self, id: &str) -> Option<&ContentTypeConfig> {
		self.content_types
			.iter()
			.find(|content_type| content_type.id == id)
	}
}

impl Default for AppConfig {
	fn default() -> Self {
		Self {
			storage: StorageConfig {
				server_root: PathBuf::new(),
				server_install_mode: ServerInstallMode::Local,
				server_ftp: FtpConfig::default(),
				server_sftp: SftpConfig::default(),
				fastdl_root: None,
				compressed_formats: vec![CompressedFormat::Gz],
				allow_overwrite: false,
				backup_existing: true,
				ftp: FtpConfig::default(),
				sftp: SftpConfig::default(),
			},
			content_types: vec![
				ContentTypeConfig {
					id: "map".to_string(),
					name: "Map".to_string(),
					max_compressed_bytes: 256 * 1024 * 1024,
					max_uncompressed_bytes: 1024 * 1024 * 1024,
					max_file_bytes: 512 * 1024 * 1024,
					max_file_count: 500,
					max_depth: 8,
					require_lowercase_paths: true,
					required_extensions: vec![".bsp".to_string()],
					required_any_extensions: Vec::new(),
					allowed_extensions: normalize_extensions(&[
						".bsp", ".res", ".txt", ".wad", ".mdl", ".wav", ".mp3", ".spr", ".tga",
						".bmp",
					]),
					path_rules: vec![
						PathRule {
							prefix: "maps".to_string(),
							extensions: normalize_extensions(&[".bsp", ".res", ".txt"]),
						},
						PathRule {
							prefix: "models".to_string(),
							extensions: normalize_extensions(&[".mdl"]),
						},
						PathRule {
							prefix: "sound".to_string(),
							extensions: normalize_extensions(&[".wav", ".mp3"]),
						},
						PathRule {
							prefix: "sprites".to_string(),
							extensions: normalize_extensions(&[".spr"]),
						},
						PathRule {
							prefix: "gfx".to_string(),
							extensions: normalize_extensions(&[".tga", ".bmp"]),
						},
					],
				},
				ContentTypeConfig {
					id: "sound".to_string(),
					name: "Sound".to_string(),
					max_compressed_bytes: 128 * 1024 * 1024,
					max_uncompressed_bytes: 512 * 1024 * 1024,
					max_file_bytes: 128 * 1024 * 1024,
					max_file_count: 500,
					max_depth: 8,
					require_lowercase_paths: true,
					required_extensions: Vec::new(),
					required_any_extensions: normalize_extensions(&[".wav", ".mp3"]),
					allowed_extensions: normalize_extensions(&[".wav", ".mp3"]),
					path_rules: vec![PathRule {
						prefix: "sound".to_string(),
						extensions: normalize_extensions(&[".wav", ".mp3"]),
					}],
				},
				ContentTypeConfig {
					id: "sprite".to_string(),
					name: "Sprite".to_string(),
					max_compressed_bytes: 128 * 1024 * 1024,
					max_uncompressed_bytes: 512 * 1024 * 1024,
					max_file_bytes: 128 * 1024 * 1024,
					max_file_count: 500,
					max_depth: 8,
					require_lowercase_paths: true,
					required_extensions: Vec::new(),
					required_any_extensions: normalize_extensions(&[".spr"]),
					allowed_extensions: normalize_extensions(&[".spr"]),
					path_rules: vec![PathRule {
						prefix: "sprites".to_string(),
						extensions: normalize_extensions(&[".spr"]),
					}],
				},
			],
		}
	}
}

fn normalize_extensions(extensions: &[&str]) -> Vec<String> {
	extensions
		.iter()
		.map(|extension| {
			let extension = extension.trim().to_ascii_lowercase();
			if extension.starts_with('.') {
				extension
			} else {
				format!(".{extension}")
			}
		})
		.collect()
}
