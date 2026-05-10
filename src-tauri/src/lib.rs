mod audit;
mod commands;
pub mod config;
mod ftp;
mod secrets;
mod sftp;
pub mod storage;
pub mod validation;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	tauri::Builder::default()
		.plugin(tauri_plugin_dialog::init())
		.invoke_handler(tauri::generate_handler![
			commands::default_config,
			commands::get_log_snapshot,
			commands::install_package,
			commands::inspect_sftp_host,
			commands::list_uploads,
			commands::load_config,
			commands::open_logs_folder,
			commands::rollback_upload,
			commands::save_config,
			commands::test_ftp_connection,
			commands::test_sftp_connection,
			commands::validate_package,
		])
		.run(tauri::generate_context!())
		.expect("failed to run FastDL Desktop");
}
