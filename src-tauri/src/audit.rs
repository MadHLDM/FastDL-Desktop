use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use time::OffsetDateTime;

pub fn append_audit(root: &Path, action: &str, status: &str, detail: &str) -> std::io::Result<()> {
	let audit_dir = root.join(".fastdl-desktop").join("logs");
	fs::create_dir_all(&audit_dir)?;
	let audit_path = audit_dir.join("audit.tsv");
	let timestamp = OffsetDateTime::now_utc()
		.format(&time::format_description::well_known::Rfc3339)
		.unwrap_or_else(|_| "unknown-time".to_string());
	let clean_detail = detail.replace(['\t', '\r', '\n'], " ");
	let mut handle = OpenOptions::new()
		.create(true)
		.append(true)
		.open(audit_path)?;
	writeln!(handle, "{timestamp}\t{action}\t{status}\t{clean_detail}")?;
	Ok(())
}
