use fastdl_desktop_lib::config::{AppConfig, CompressedFormat};
use fastdl_desktop_lib::storage::{install_package, list_uploads, rollback_upload, UploadManifest};
use fastdl_desktop_lib::validation::validate_zip;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

fn test_config(server_root: &Path) -> AppConfig {
	let mut config = AppConfig::default();
	config.storage.server_root = server_root.to_path_buf();
	config.storage.compressed_formats = vec![CompressedFormat::Gz, CompressedFormat::Bz2];
	config
}

fn test_config_with_fastdl(server_root: &Path, fastdl_root: &Path) -> AppConfig {
	let mut config = test_config(server_root);
	config.storage.fastdl_root = Some(fastdl_root.to_path_buf());
	config
}

fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
	let file = File::create(path).expect("create zip");
	let mut zip = ZipWriter::new(file);
	let options = SimpleFileOptions::default();
	for (name, content) in files {
		zip.start_file(*name, options).expect("start zip file");
		zip.write_all(content).expect("write zip file");
	}
	zip.finish().expect("finish zip");
}

fn temp_zip(temp: &TempDir, files: &[(&str, &[u8])]) -> PathBuf {
	let zip_path = temp.path().join("package.zip");
	write_zip(&zip_path, files);
	zip_path
}

fn write_tar_gz(path: &Path, files: &[(&str, &[u8])]) {
	let file = File::create(path).expect("create tar.gz");
	let encoder = GzEncoder::new(file, Compression::default());
	let mut archive = tar::Builder::new(encoder);
	for (name, content) in files {
		let mut header = tar::Header::new_gnu();
		header.set_size(content.len() as u64);
		header.set_cksum();
		archive
			.append_data(&mut header, *name, *content)
			.expect("append tar file");
	}
	archive.finish().expect("finish tar.gz");
}

fn temp_tar_gz(temp: &TempDir, files: &[(&str, &[u8])]) -> PathBuf {
	let path = temp.path().join("package.tar.gz");
	write_tar_gz(&path, files);
	path
}

#[test]
fn validates_safe_map_package() {
	let temp = TempDir::new().unwrap();
	let config = test_config(temp.path());
	let zip_path = temp_zip(
		&temp,
		&[
			("maps/test.bsp", b"bsp"),
			("maps/test.res", b"sound/test/ambience.wav\n"),
			("sound/test/ambience.wav", b"wav"),
		],
	);

	let report = validate_zip(&config, &zip_path, "map").expect("valid package");

	assert_eq!(report.file_count, 3);
	assert!(report
		.compressed_files
		.iter()
		.any(|path| path == "maps/test.bsp.gz"));
	assert!(report
		.compressed_files
		.iter()
		.any(|path| path == "maps/test.bsp.bz2"));
}

#[test]
fn validates_and_installs_tar_gz_map_package() {
	let temp = TempDir::new().unwrap();
	let server_root = temp.path().join("server");
	let config = test_config(&server_root);
	let package_path = temp_tar_gz(
		&temp,
		&[
			("maps/test.bsp", b"bsp"),
			("maps/test.res", b"sound/test.wav\n"),
			("sound/test.wav", b"wav"),
		],
	);

	let report = validate_zip(&config, &package_path, "map").expect("valid tar.gz package");
	assert_eq!(report.file_count, 3);

	install_package(&config, &package_path, "map").expect("install tar.gz package");
	assert!(server_root.join("maps/test.bsp").exists());
}

#[test]
fn rejects_path_traversal_backslash_drive_and_uppercase() {
	let cases = [
		("../maps/test.bsp", "path traversal"),
		("maps\\test.bsp", "folder separator"),
		("C:/maps/test.bsp", "drive paths"),
		("Maps/test.bsp", "lowercase"),
	];

	for (entry, expected) in cases {
		let temp = TempDir::new().unwrap();
		let config = test_config(temp.path());
		let zip_path = temp_zip(&temp, &[(entry, b"bad")]);
		let error = validate_zip(&config, &zip_path, "map")
			.expect_err("unsafe path should fail")
			.to_string();
		assert!(
			error.contains(expected),
			"expected {expected:?} in error for {entry:?}, got {error:?}"
		);
	}
}

#[test]
fn rejects_case_collision() {
	let temp = TempDir::new().unwrap();
	let mut config = test_config(temp.path());
	config.content_types[0].require_lowercase_paths = false;
	let zip_path = temp_zip(
		&temp,
		&[("maps/test.bsp", b"one"), ("maps/TEST.bsp", b"two")],
	);

	let error = validate_zip(&config, &zip_path, "map")
		.expect_err("case collision should fail")
		.to_string();

	assert!(error.contains("case collision"));
}

#[test]
fn rejects_forbidden_extension_and_folder() {
	let temp = TempDir::new().unwrap();
	let config = test_config(temp.path());
	let bad_extension = temp_zip(&temp, &[("maps/test.exe", b"bad")]);
	let error = validate_zip(&config, &bad_extension, "map")
		.expect_err("extension should fail")
		.to_string();
	assert!(error.contains("extension .exe is not allowed"));

	let bad_folder = temp.path().join("bad-folder.zip");
	write_zip(&bad_folder, &[("scripts/test.bsp", b"bad")]);
	let error = validate_zip(&config, &bad_folder, "map")
		.expect_err("folder should fail")
		.to_string();
	assert!(error.contains("folder/extension is not allowed"));
}

#[test]
fn validates_res_resources_against_zip_and_server_root() {
	let temp = TempDir::new().unwrap();
	let server_file = temp.path().join("sound/existing.wav");
	fs::create_dir_all(server_file.parent().unwrap()).unwrap();
	fs::write(&server_file, b"server").unwrap();
	let config = test_config(temp.path());
	let zip_path = temp_zip(
		&temp,
		&[
			("maps/test.bsp", b"bsp"),
			("maps/test.res", b"sound/existing.wav\nsound/inzip.wav\n"),
			("sound/inzip.wav", b"zip"),
		],
	);

	validate_zip(&config, &zip_path, "map").expect("res should resolve");
}

#[test]
fn rejects_missing_res_resource() {
	let temp = TempDir::new().unwrap();
	let config = test_config(temp.path());
	let zip_path = temp_zip(
		&temp,
		&[
			("maps/test.bsp", b"bsp"),
			("maps/test.res", b"sound/missing.wav\n"),
		],
	);

	let error = validate_zip(&config, &zip_path, "map")
		.expect_err("missing res resource should fail")
		.to_string();

	assert!(error.contains("listed resource is missing"));
}

#[test]
fn installs_compresses_writes_manifest_and_rolls_back() {
	let temp = TempDir::new().unwrap();
	let server_root = temp.path().join("server");
	let fastdl_root = temp.path().join("fastdl");
	let config = test_config_with_fastdl(&server_root, &fastdl_root);
	let zip_path = temp_zip(
		&temp,
		&[
			("maps/test.bsp", b"bsp"),
			("maps/test.res", b"sound/test.wav\n"),
			("sound/test.wav", b"wav"),
		],
	);

	let report = install_package(&config, &zip_path, "map").expect("install package");

	assert!(server_root.join("maps/test.bsp").exists());
	assert!(fastdl_root.join("maps/test.bsp.gz").exists());
	assert!(fastdl_root.join("maps/test.bsp.bz2").exists());
	assert!(server_root
		.join(".uploads")
		.join(format!("{}.json", report.upload_id))
		.exists());
	assert!(server_root.join(".fastdl-desktop/logs/audit.tsv").exists());
	let manifests = list_uploads(&config).expect("list uploads");
	assert_eq!(manifests.len(), 1);
	assert_eq!(manifests[0].status, "installed");

	let rollback = rollback_upload(&config, &report.upload_id, true).expect("rollback upload");

	assert!(rollback
		.deleted_files
		.iter()
		.any(|path| path == "maps/test.bsp"));
	assert!(!server_root.join("maps/test.bsp").exists());
	assert!(!fastdl_root.join("maps/test.bsp.gz").exists());
	let manifests = list_uploads(&config).expect("list uploads after rollback");
	assert_eq!(manifests[0].status, "rolled_back");
}

#[test]
fn rollback_refuses_modified_installed_file() {
	let temp = TempDir::new().unwrap();
	let server_root = temp.path().join("server");
	let config = test_config(&server_root);
	let zip_path = temp_zip(&temp, &[("maps/test.bsp", b"bsp"), ("maps/test.res", b"")]);
	let report = install_package(&config, &zip_path, "map").expect("install package");
	fs::write(server_root.join("maps/test.bsp"), b"modified").unwrap();

	let error = rollback_upload(&config, &report.upload_id, true)
		.expect_err("modified file should block rollback")
		.to_string();

	assert!(error.contains("modified file"));
	assert!(server_root.join("maps/test.bsp").exists());
}

#[test]
fn rollback_requires_remote_protocol_for_published_files() {
	let temp = TempDir::new().unwrap();
	let server_root = temp.path().join("server");
	let config = test_config(&server_root);
	let zip_path = temp_zip(&temp, &[("maps/test.bsp", b"bsp"), ("maps/test.res", b"")]);
	let report = install_package(&config, &zip_path, "map").expect("install package");
	let manifest_path = server_root
		.join(".uploads")
		.join(format!("{}.json", report.upload_id));
	let mut manifest_json =
		serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&manifest_path).unwrap())
			.unwrap();
	manifest_json["ftpPublishedFiles"] = serde_json::Value::Array(vec![serde_json::Value::String(
		"maps/test.bsp.gz".to_string(),
	)]);
	fs::write(
		&manifest_path,
		serde_json::to_string_pretty(&manifest_json).unwrap(),
	)
	.unwrap();

	let error = rollback_upload(&config, &report.upload_id, true)
		.expect_err("remote published files should require remote rollback")
		.to_string();

	assert!(error.contains("needs FTP enabled"));
	assert!(server_root.join("maps/test.bsp").exists());
}

#[test]
fn reads_legacy_manifest_without_remote_rollback_fields() {
	let manifest = serde_json::from_str::<UploadManifest>(
		r#"{
			"uploadId": "legacy",
			"contentType": "map",
			"sourceZip": "package.zip",
			"status": "installed",
			"startedAt": "2026-01-01T00:00:00Z",
			"completedAt": "2026-01-01T00:00:01Z",
			"rolledBackAt": null,
			"serverRoot": "server",
			"fastdlRoot": null,
			"installedFiles": [],
			"installedHashes": [],
			"compressedFiles": [],
			"compressedHashes": [],
			"backups": [],
			"ftpPublishedFiles": ["maps/test.bsp.gz"],
			"sftpPublishedFiles": []
		}"#,
	)
	.expect("legacy manifest should deserialize");

	assert_eq!(manifest.ftp_rolled_back_files, Vec::<String>::new());
	assert_eq!(manifest.sftp_rolled_back_files, Vec::<String>::new());
}
