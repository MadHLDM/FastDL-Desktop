import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import "./styles.css";

type CompressedFormat = "gz" | "bz2";

type AppConfig = {
	storage: {
		serverRoot: string;
		serverInstallMode: "local" | "ftp" | "sftp";
		serverFtp: {
			enabled: boolean;
			host: string | null;
			port: number;
			username: string | null;
			password: string | null;
			remoteFastdlRoot: string | null;
		};
		serverSftp: {
			enabled: boolean;
			host: string | null;
			port: number;
			username: string | null;
			password: string | null;
			privateKeyPath: string | null;
			privateKeyPassphrase: string | null;
			remoteFastdlRoot: string | null;
			trustedHostFingerprint: string | null;
		};
		fastdlRoot: string | null;
		compressedFormats: CompressedFormat[];
		allowOverwrite: boolean;
		backupExisting: boolean;
		ftp: {
			enabled: boolean;
			host: string | null;
			port: number;
			username: string | null;
			password: string | null;
			remoteFastdlRoot: string | null;
		};
		sftp: {
			enabled: boolean;
			host: string | null;
			port: number;
			username: string | null;
			password: string | null;
			privateKeyPath: string | null;
			privateKeyPassphrase: string | null;
			remoteFastdlRoot: string | null;
			trustedHostFingerprint: string | null;
		};
	};
	contentTypes: ContentType[];
};

type ContentType = {
	id: string;
	name: string;
};

type CountItem = {
	name: string;
	count: number;
};

type FileSummary = {
	path: string;
	size: number;
	compressedSize: number;
};

type ValidationReport = {
	contentType: string;
	fileCount: number;
	totalUncompressedBytes: number;
	folders: CountItem[];
	extensions: CountItem[];
	largestFiles: FileSummary[];
	files: FileSummary[];
	compressedFiles: string[];
	conflicts: string[];
};

type UploadManifest = {
	uploadId: string;
	contentType: string;
	status: string;
	startedAt: string;
	completedAt: string | null;
	installedFiles: string[];
	serverPublishedFiles: string[];
	serverRolledBackFiles: string[];
	compressedFiles: string[];
	ftpPublishedFiles: string[];
	sftpPublishedFiles: string[];
	ftpRolledBackFiles: string[];
	sftpRolledBackFiles: string[];
};

type AuditEntry = {
	timestamp: string;
	action: string;
	status: string;
	detail: string;
};

type LogSnapshot = {
	logsDirectory: string;
	manifestsDirectory: string;
	auditEntries: AuditEntry[];
	manifests: UploadManifest[];
};

type SftpHostFingerprint = {
	host: string;
	port: number;
	hostKeyType: string;
	sha256: string;
};

type InstallReport = {
	uploadId: string;
	validation: ValidationReport;
	installedFiles: string[];
	serverPublishedFiles: string[];
	compressedFiles: string[];
	ftpPublishedFiles: string[];
	sftpPublishedFiles: string[];
	ftpDeletedFiles: string[];
	sftpDeletedFiles: string[];
	serverDeletedFiles: string[];
};

type ProgressEvent = {
	step: string;
	message: string;
	current: number;
	total: number;
};

const app = document.querySelector<HTMLDivElement>("#app");

if (!app) {
	throw new Error("App root was not found");
}

const appRoot = app;

let config: AppConfig | null = null;
let selectedZipPath = "";
let selectedContentType = "";
let validationReport: ValidationReport | null = null;
let installReport: InstallReport | null = null;
let uploadHistory: UploadManifest[] = [];
let logSnapshot: LogSnapshot | null = null;
let logViewerLoading = false;
let progressEvents: ProgressEvent[] = [];
let busy = false;
let statusMessage = "Loading configuration";
let errorMessage = "";
let configSaveTimer: number | null = null;
let configDirty = false;
let configSaveInFlight = false;
let configSaveQueued = false;
let configVersion = 0;
let savedConfigVersion = 0;

void boot();
void listen("install-progress", (event) => {
	const progress = event.payload as ProgressEvent;
	progressEvents = [...progressEvents.slice(-7), progress];
	statusMessage = progress.message;
	render();
});

async function boot() {
	try {
		config = await invoke<AppConfig>("load_config");
		selectedContentType = config.contentTypes[0]?.id ?? "";
		await loadUploads();
		await loadLogSnapshot();
		statusMessage = "Ready";
	} catch (error) {
		errorMessage = formatError(error);
		statusMessage = "Error";
	}
	render();
}

function render() {
	if (!config) {
		appRoot.innerHTML = `
			<main class="app-shell">
				<section class="workspace">
					<p class="status-line">${escapeHtml(statusMessage)}</p>
				</section>
			</main>
		`;
		return;
	}

	appRoot.innerHTML = `
		<main class="app-shell">
			<header class="topbar">
				<div>
					<p class="eyebrow">FastDL Desktop</p>
					<h1>Sven Co-op Packages</h1>
				</div>
				<div class="status-pill ${errorMessage ? "is-error" : ""}" title="${escapeAttribute(errorMessage || statusMessage)}">${escapeHtml(errorMessage || statusMessage)}</div>
			</header>

			<section class="workspace">
				<aside class="sidebar" aria-label="Configuration">
					<label class="field">
						<span>Server root</span>
						<div class="path-row">
							<input id="server-root" value="${escapeAttribute(config.storage.serverRoot)}" spellcheck="false" />
							<button id="pick-server-root" class="icon-button" title="Select server directory" type="button">...</button>
						</div>
					</label>

					${renderServerDestination(config)}

					<label class="field">
						<span>FastDL</span>
						<div class="path-row">
							<input id="fastdl-root" value="${escapeAttribute(config.storage.fastdlRoot ?? "")}" spellcheck="false" />
							<button id="pick-fastdl-root" class="icon-button" title="Select FastDL directory" type="button">...</button>
						</div>
					</label>

					<div class="field">
						<span>Compression</span>
						<div class="segmented" role="group" aria-label="Compression formats">
							<label>
								<input id="format-gz" type="checkbox" ${config.storage.compressedFormats.includes("gz") ? "checked" : ""} />
								<span>GZ</span>
							</label>
							<label>
								<input id="format-bz2" type="checkbox" ${config.storage.compressedFormats.includes("bz2") ? "checked" : ""} />
								<span>BZ2</span>
							</label>
						</div>
					</div>

					<label class="toggle">
						<input id="backup-existing" type="checkbox" ${config.storage.backupExisting ? "checked" : ""} />
						<span>Back up before replacing</span>
					</label>
					<p class="field-help">Stores local replacements under .backups for rollback.</p>

					<label class="toggle">
						<input id="allow-overwrite" type="checkbox" ${config.storage.allowOverwrite ? "checked" : ""} />
						<span>Allow overwrite</span>
					</label>
					<p class="field-help">Required before replacing existing destination files.</p>
					${renderOverwriteWarning(config)}

					${renderRemotePublishing(config)}
				</aside>

				<section class="main-panel">
					<div class="upload-strip">
						<label class="field compact">
							<span>Type</span>
							<select id="content-type">
								${config.contentTypes.map((contentType) => `
									<option value="${escapeAttribute(contentType.id)}" ${contentType.id === selectedContentType ? "selected" : ""}>
										${escapeHtml(contentType.name)}
									</option>
								`).join("")}
							</select>
						</label>

						<label class="field grow">
							<span>Package</span>
							<div class="path-row">
								<input id="zip-path" value="${escapeAttribute(selectedZipPath)}" spellcheck="false" />
								<button id="pick-zip" class="icon-button" title="Select package archive" type="button">...</button>
							</div>
						</label>

						<button id="validate" class="primary-button" type="button" ${busy ? "disabled" : ""}>
							${busy ? "Validating" : "Validate"}
						</button>
						<button id="install" class="secondary-button" type="button" ${busy || !validationReport || hasBlockingConflicts() ? "disabled" : ""}>
							Install
						</button>
					</div>

					${validationReport ? `${renderDeploymentPlan(validationReport)}${renderReport(validationReport)}` : renderEmptyState()}
					${renderProgress()}
					${installReport ? renderInstallReport(installReport) : ""}
					${renderHistory()}
					${renderLogViewer()}
				</section>
			</section>
		</main>
	`;

	bindEvents();
}

function renderProgress() {
	if (!busy && !progressEvents.length) {
		return "";
	}
	const latest = progressEvents.length ? progressEvents[progressEvents.length - 1] : null;
	return `
		<section class="table-panel wide progress-panel">
			<div class="panel-header">
				<h2>Progress</h2>
				<span>${latest ? escapeHtml(latest.step) : ""}</span>
			</div>
			${latest ? `
				<div class="progress-current">
					<div>
						<strong>${escapeHtml(latest.message)}</strong>
						<span>${latest.total ? `${latest.current} / ${latest.total}` : "Working"}</span>
					</div>
					${latest.total ? `<progress value="${latest.current}" max="${latest.total}"></progress>` : `<progress></progress>`}
				</div>
			` : ""}
			<div class="progress-log">
				${progressEvents.map((item) => `
					<div>
						<span>${escapeHtml(item.step)}</span>
						<strong>${escapeHtml(item.message)}</strong>
					</div>
				`).join("")}
			</div>
		</section>
	`;
}

function renderInstallReport(report: InstallReport) {
	return `
		<section class="table-panel wide install-panel">
			<div class="panel-header">
				<h2>Installed Upload</h2>
				<span>${escapeHtml(report.uploadId)}</span>
			</div>
			<div class="summary-list">
				<div><span>Installed files</span><strong>${report.installedFiles.length}</strong></div>
				<div><span>Game server published</span><strong>${report.serverPublishedFiles.length}</strong></div>
				<div><span>Compressed files</span><strong>${report.compressedFiles.length}</strong></div>
				<div><span>FTP published files</span><strong>${report.ftpPublishedFiles.length}</strong></div>
				<div><span>SFTP published files</span><strong>${report.sftpPublishedFiles.length}</strong></div>
			</div>
		</section>
	`;
}

function renderHistory() {
	return `
		<section class="table-panel wide history-panel">
			<div class="panel-header">
				<h2>Upload History</h2>
				<span>${uploadHistory.length}</span>
			</div>
			${uploadHistory.length ? `
				<div class="history-list">
					${uploadHistory.map((upload) => `
						<div class="history-item">
							<div class="history-row">
								<div>
									<strong>${escapeHtml(upload.uploadId)}</strong>
									<span>${escapeHtml(upload.contentType)} / ${escapeHtml(upload.status)} / ${escapeHtml(upload.completedAt ?? upload.startedAt)}</span>
									<div class="history-badges">
										<span>${upload.installedFiles.length} installed</span>
										${renderRemoteBadge("Game server", upload.serverPublishedFiles?.length ?? 0)}
										<span>${upload.compressedFiles.length} compressed</span>
										${renderRemoteBadge("FTP", upload.ftpPublishedFiles.length)}
										${renderRemoteBadge("SFTP", upload.sftpPublishedFiles.length)}
										${renderRemoteRollbackBadge("FTP rollback", upload.ftpRolledBackFiles?.length ?? 0)}
										${renderRemoteRollbackBadge("SFTP rollback", upload.sftpRolledBackFiles?.length ?? 0)}
										${renderRemoteRollbackBadge("Server rollback", upload.serverRolledBackFiles?.length ?? 0)}
									</div>
								</div>
								<button class="rollback-button" data-upload-id="${escapeAttribute(upload.uploadId)}" type="button" ${busy || upload.status === "rolled_back" ? "disabled" : ""}>
									Roll back
								</button>
							</div>
							${renderRemotePublishHistory(upload)}
						</div>
					`).join("")}
				</div>
			` : `<p class="quiet">No uploads yet.</p>`}
		</section>
	`;
}

function renderLogViewer() {
	const auditEntries = logSnapshot?.auditEntries ?? [];
	const manifests = logSnapshot?.manifests ?? [];
	return `
		<section class="table-panel wide log-panel">
			<div class="panel-header log-header">
				<div>
					<h2>Log Viewer</h2>
					<span>${logSnapshot ? `${auditEntries.length} audit entries / ${manifests.length} manifests` : "Audit and manifests"}</span>
				</div>
				<div class="panel-actions">
					<button id="refresh-logs" class="secondary-button compact-button" type="button" ${busy || logViewerLoading ? "disabled" : ""}>
						${logViewerLoading ? "Refreshing" : "Refresh"}
					</button>
					<button id="open-logs-folder" class="secondary-button compact-button" type="button" ${busy || logViewerLoading ? "disabled" : ""}>
						Open folder
					</button>
				</div>
			</div>
			${logSnapshot ? `
				<div class="log-paths">
					<div title="${escapeAttribute(logSnapshot.logsDirectory)}">
						<span>Logs</span>
						<strong>${escapeHtml(logSnapshot.logsDirectory)}</strong>
					</div>
					<div title="${escapeAttribute(logSnapshot.manifestsDirectory)}">
						<span>Manifests</span>
						<strong>${escapeHtml(logSnapshot.manifestsDirectory)}</strong>
					</div>
				</div>
			` : `<p class="quiet">No log snapshot loaded.</p>`}
			<div class="log-grid">
				<section class="log-section">
					<div class="subheader">
						<h3>Audit</h3>
						<span>${auditEntries.length}</span>
					</div>
					${renderAuditEntries(auditEntries)}
				</section>
				<section class="log-section">
					<div class="subheader">
						<h3>Manifests</h3>
						<span>${manifests.length}</span>
					</div>
					${renderManifestEntries(manifests)}
				</section>
			</div>
		</section>
	`;
}

function renderAuditEntries(entries: AuditEntry[]) {
	if (!entries.length) {
		return `<p class="quiet">No audit entries yet.</p>`;
	}
	return `
		<div class="audit-list">
			${entries.map((entry) => `
				<div class="audit-row">
					<span title="${escapeAttribute(entry.timestamp)}">${escapeHtml(entry.timestamp)}</span>
					<strong>${escapeHtml(entry.action)}</strong>
					<em>${escapeHtml(entry.status)}</em>
					<p title="${escapeAttribute(entry.detail)}">${escapeHtml(entry.detail)}</p>
				</div>
			`).join("")}
		</div>
	`;
}

function renderManifestEntries(manifests: UploadManifest[]) {
	if (!manifests.length) {
		return `<p class="quiet">No manifests yet.</p>`;
	}
	return `
		<div class="manifest-list">
			${manifests.map((manifest) => `
				<details class="manifest-entry">
					<summary>
						<div>
							<strong>${escapeHtml(manifest.uploadId)}</strong>
							<span>${escapeHtml(manifest.contentType)} / ${escapeHtml(manifest.status)} / ${escapeHtml(manifest.completedAt ?? manifest.startedAt)}</span>
						</div>
						<em>${manifest.installedFiles.length + manifest.compressedFiles.length} files</em>
					</summary>
					<div class="manifest-details">
						<div><span>Started</span><strong>${escapeHtml(manifest.startedAt)}</strong></div>
						<div><span>Completed</span><strong>${escapeHtml(manifest.completedAt ?? "Not completed")}</strong></div>
						<div><span>Installed</span><strong>${manifest.installedFiles.length}</strong></div>
						<div><span>Compressed</span><strong>${manifest.compressedFiles.length}</strong></div>
						<div><span>Remote</span><strong>${(manifest.serverPublishedFiles?.length ?? 0) + manifest.ftpPublishedFiles.length + manifest.sftpPublishedFiles.length}</strong></div>
					</div>
					${renderRemotePublishHistory(manifest)}
				</details>
			`).join("")}
		</div>
	`;
}

function renderRemoteBadge(protocol: string, count: number) {
	if (!count) {
		return "";
	}
	return `<span class="remote-badge">${escapeHtml(protocol)} ${count}</span>`;
}

function renderRemoteRollbackBadge(label: string, count: number) {
	if (!count) {
		return "";
	}
	return `<span class="remote-rollback-badge">${escapeHtml(label)} ${count}</span>`;
}

function renderRemotePublishHistory(upload: UploadManifest) {
	const groups = [
		{
			protocol: "Game server",
			files: upload.serverPublishedFiles ?? [],
		},
		{
			protocol: "FTP",
			files: upload.ftpPublishedFiles,
		},
		{
			protocol: "SFTP",
			files: upload.sftpPublishedFiles,
		},
		{
			protocol: "FTP rolled back",
			files: upload.ftpRolledBackFiles ?? [],
		},
		{
			protocol: "SFTP rolled back",
			files: upload.sftpRolledBackFiles ?? [],
		},
		{
			protocol: "Game server rolled back",
			files: upload.serverRolledBackFiles ?? [],
		},
	].filter((group) => group.files.length > 0);

	if (!groups.length) {
		return "";
	}

	return `
		<details class="remote-history">
			<summary>
				<span>Remote publishing</span>
				<strong>${groups.map((group) => `${group.protocol} ${group.files.length}`).join(" / ")}</strong>
			</summary>
			<div class="remote-history-groups">
				${groups.map((group) => `
					<section>
						<h3>${escapeHtml(group.protocol)}</h3>
						<div class="remote-file-list">
							${group.files.map((path) => `<span title="${escapeAttribute(path)}">${escapeHtml(path)}</span>`).join("")}
						</div>
					</section>
				`).join("")}
			</div>
		</details>
	`;
}

function renderEmptyState() {
	return `
		<div class="empty-state">
			<div class="empty-mark">FDL</div>
			<h2>No package validated</h2>
			<p>Select an archive to generate the preview before installing.</p>
		</div>
	`;
}

function renderServerDestination(currentConfig: AppConfig) {
	const mode = currentConfig.storage.serverInstallMode;
	return `
		<div class="field">
			<span>Game server install</span>
			<select id="server-install-mode">
				<option value="local" ${mode === "local" ? "selected" : ""}>Local only</option>
				<option value="ftp" ${mode === "ftp" ? "selected" : ""}>Local + FTP</option>
				<option value="sftp" ${mode === "sftp" ? "selected" : ""}>Local + SFTP</option>
			</select>
		</div>

		${mode === "ftp" ? renderServerFtpFields(currentConfig) : ""}
		${mode === "sftp" ? renderServerSftpFields(currentConfig) : ""}
	`;
}

function renderServerFtpFields(currentConfig: AppConfig) {
	return `
		<label class="field">
			<span>Server FTP host</span>
			<input id="server-ftp-host" value="${escapeAttribute(currentConfig.storage.serverFtp.host ?? "")}" spellcheck="false" />
		</label>

		<div class="split-fields">
			<label class="field">
				<span>Port</span>
				<input id="server-ftp-port" value="${currentConfig.storage.serverFtp.port}" inputmode="numeric" />
			</label>
			<label class="field">
				<span>User</span>
				<input id="server-ftp-username" value="${escapeAttribute(currentConfig.storage.serverFtp.username ?? "")}" spellcheck="false" />
			</label>
		</div>

		<label class="field">
			<span>Password</span>
			<input id="server-ftp-password" type="password" value="${escapeAttribute(currentConfig.storage.serverFtp.password ?? "")}" />
		</label>

		<label class="field">
			<span>Remote server root</span>
			<input id="server-ftp-remote-root" value="${escapeAttribute(currentConfig.storage.serverFtp.remoteFastdlRoot ?? "")}" spellcheck="false" />
		</label>

		<button id="test-server-ftp" class="secondary-button full-width" type="button">Test Game Server FTP</button>
	`;
}

function renderServerSftpFields(currentConfig: AppConfig) {
	return `
		<label class="field">
			<span>Server SFTP host</span>
			<input id="server-sftp-host" value="${escapeAttribute(currentConfig.storage.serverSftp.host ?? "")}" spellcheck="false" />
		</label>

		<div class="split-fields">
			<label class="field">
				<span>Port</span>
				<input id="server-sftp-port" value="${currentConfig.storage.serverSftp.port}" inputmode="numeric" />
			</label>
			<label class="field">
				<span>User</span>
				<input id="server-sftp-username" value="${escapeAttribute(currentConfig.storage.serverSftp.username ?? "")}" spellcheck="false" />
			</label>
		</div>

		<label class="field">
			<span>Password</span>
			<input id="server-sftp-password" type="password" value="${escapeAttribute(currentConfig.storage.serverSftp.password ?? "")}" />
		</label>

		<label class="field">
			<span>Private key</span>
			<div class="path-row">
				<input id="server-sftp-private-key" value="${escapeAttribute(currentConfig.storage.serverSftp.privateKeyPath ?? "")}" spellcheck="false" />
				<button id="pick-server-sftp-key" class="icon-button" title="Select game server private key" type="button">...</button>
			</div>
		</label>

		<label class="field">
			<span>Key passphrase</span>
			<input id="server-sftp-key-passphrase" type="password" value="${escapeAttribute(currentConfig.storage.serverSftp.privateKeyPassphrase ?? "")}" />
		</label>

		<label class="field">
			<span>Remote server root</span>
			<input id="server-sftp-remote-root" value="${escapeAttribute(currentConfig.storage.serverSftp.remoteFastdlRoot ?? "")}" spellcheck="false" />
		</label>

		${renderTrustedSftpFingerprint(currentConfig.storage.serverSftp.trustedHostFingerprint)}

		<button id="test-server-sftp" class="secondary-button full-width" type="button">Test Game Server SFTP</button>
	`;
}

function renderRemotePublishing(currentConfig: AppConfig) {
	const mode = remoteMode(currentConfig);
	return `
		<div class="divider"></div>

		<section class="remote-section">
			<div class="section-heading">
				<span>Remote Publishing</span>
			</div>

			<label class="field">
				<span>Protocol</span>
				<select id="remote-mode">
					<option value="none" ${mode === "none" ? "selected" : ""}>None</option>
					<option value="ftp" ${mode === "ftp" ? "selected" : ""}>FTP</option>
					<option value="sftp" ${mode === "sftp" ? "selected" : ""}>SFTP</option>
				</select>
			</label>

			${mode === "ftp" ? renderFtpFields(currentConfig) : ""}
			${mode === "sftp" ? renderSftpFields(currentConfig) : ""}
		</section>
	`;
}

function renderOverwriteWarning(currentConfig: AppConfig) {
	if (!currentConfig.storage.allowOverwrite || currentConfig.storage.backupExisting) {
		return "";
	}
	return `
		<div class="inline-warning">
			<strong>Overwrite without backup</strong>
			<span>Existing local files will be replaced without a restore copy.</span>
		</div>
	`;
}

function renderFtpFields(currentConfig: AppConfig) {
	return `
		<label class="field">
			<span>FTP host</span>
			<input id="ftp-host" value="${escapeAttribute(currentConfig.storage.ftp.host ?? "")}" spellcheck="false" />
		</label>

		<div class="split-fields">
			<label class="field">
				<span>Port</span>
				<input id="ftp-port" value="${currentConfig.storage.ftp.port}" inputmode="numeric" />
			</label>
			<label class="field">
				<span>User</span>
				<input id="ftp-username" value="${escapeAttribute(currentConfig.storage.ftp.username ?? "")}" spellcheck="false" />
			</label>
		</div>

		<label class="field">
			<span>Password</span>
			<input id="ftp-password" type="password" value="${escapeAttribute(currentConfig.storage.ftp.password ?? "")}" />
		</label>

		<label class="field">
			<span>Remote FastDL root</span>
			<input id="ftp-remote-root" value="${escapeAttribute(currentConfig.storage.ftp.remoteFastdlRoot ?? "")}" spellcheck="false" />
		</label>

		<button id="test-fastdl-ftp" class="secondary-button full-width" type="button">Test FastDL FTP</button>
	`;
}

function renderSftpFields(currentConfig: AppConfig) {
	return `
		<label class="field">
			<span>SFTP host</span>
			<input id="sftp-host" value="${escapeAttribute(currentConfig.storage.sftp.host ?? "")}" spellcheck="false" />
		</label>

		<div class="split-fields">
			<label class="field">
				<span>Port</span>
				<input id="sftp-port" value="${currentConfig.storage.sftp.port}" inputmode="numeric" />
			</label>
			<label class="field">
				<span>User</span>
				<input id="sftp-username" value="${escapeAttribute(currentConfig.storage.sftp.username ?? "")}" spellcheck="false" />
			</label>
		</div>

		<label class="field">
			<span>Password</span>
			<input id="sftp-password" type="password" value="${escapeAttribute(currentConfig.storage.sftp.password ?? "")}" />
		</label>

		<label class="field">
			<span>Private key</span>
			<div class="path-row">
				<input id="sftp-private-key" value="${escapeAttribute(currentConfig.storage.sftp.privateKeyPath ?? "")}" spellcheck="false" />
				<button id="pick-sftp-key" class="icon-button" title="Select private key" type="button">...</button>
			</div>
		</label>

		<label class="field">
			<span>Key passphrase</span>
			<input id="sftp-key-passphrase" type="password" value="${escapeAttribute(currentConfig.storage.sftp.privateKeyPassphrase ?? "")}" />
		</label>

		<label class="field">
			<span>Remote FastDL root</span>
			<input id="sftp-remote-root" value="${escapeAttribute(currentConfig.storage.sftp.remoteFastdlRoot ?? "")}" spellcheck="false" />
		</label>

		${renderTrustedSftpFingerprint(currentConfig.storage.sftp.trustedHostFingerprint)}

		<button id="test-fastdl-sftp" class="secondary-button full-width" type="button">Test FastDL SFTP</button>
	`;
}

function renderTrustedSftpFingerprint(fingerprint: string | null) {
	if (!fingerprint) {
		return `
			<div class="trust-status is-untrusted">
				<strong>SFTP host not trusted</strong>
				<span>Run Test SFTP to review and save the host fingerprint.</span>
			</div>
		`;
	}
	return `
		<div class="trust-status">
			<strong>Trusted SFTP host</strong>
			<span title="${escapeAttribute(fingerprint)}">${escapeHtml(fingerprint)}</span>
		</div>
	`;
}

function renderReport(report: ValidationReport) {
	return `
		<div class="report-grid">
			<section class="metric-panel">
				<div>
					<span class="metric-label">Type</span>
					<strong>${escapeHtml(report.contentType)}</strong>
				</div>
				<div>
					<span class="metric-label">Files</span>
					<strong>${report.fileCount}</strong>
				</div>
				<div>
					<span class="metric-label">Size</span>
					<strong>${formatBytes(report.totalUncompressedBytes)}</strong>
				</div>
				<div>
					<span class="metric-label">Conflicts</span>
					<strong class="${report.conflicts.length ? "danger-text" : ""}">${report.conflicts.length}</strong>
				</div>
			</section>

			<section class="table-panel">
				<div class="panel-header">
					<h2>Largest Files</h2>
				</div>
				${renderFileTable(report.largestFiles)}
			</section>

			<section class="table-panel">
				<div class="panel-header">
					<h2>Folders</h2>
				</div>
				${renderCounts(report.folders)}
			</section>

			<section class="table-panel">
				<div class="panel-header">
					<h2>Extensions</h2>
				</div>
				${renderCounts(report.extensions)}
			</section>

			<section class="table-panel wide">
				<div class="panel-header">
					<h2>Validated Files</h2>
					<span>${report.files.length}</span>
				</div>
				${renderFileTable(report.files.slice(0, 80))}
			</section>

			<section class="table-panel wide">
				<div class="panel-header">
					<h2>Planned FastDL</h2>
					<span>${report.compressedFiles.length}</span>
				</div>
				${renderPathList(report.compressedFiles.slice(0, 80))}
			</section>

			${report.conflicts.length ? `
				<section class="table-panel wide danger-panel">
					<div class="panel-header">
						<h2>Destination Conflicts</h2>
						<span>${report.conflicts.length}</span>
					</div>
					${renderPathList(report.conflicts)}
				</section>
			` : ""}
		</div>
	`;
}

function renderDeploymentPlan(report: ValidationReport) {
	if (!config) {
		return "";
	}
	const serverRemoteCount = config.storage.serverInstallMode === "local" ? 0 : report.files.length;
	const fastdlMode = remoteMode(config);
	const fastdlRemoteCount = fastdlMode === "none" ? 0 : report.compressedFiles.length;
	return `
		<section class="table-panel wide deployment-plan">
			<div class="panel-header">
				<h2>Deployment Plan</h2>
				<span>${report.fileCount} source / ${report.compressedFiles.length} FastDL</span>
			</div>
			<div class="plan-grid">
				<div>
					<span>Local server staging</span>
					<strong>${report.files.length}</strong>
				</div>
				<div>
					<span>Remote game server</span>
					<strong>${serverRemoteCount}</strong>
					<small>${escapeHtml(serverModeLabel(config.storage.serverInstallMode))}</small>
				</div>
				<div>
					<span>Local FastDL output</span>
					<strong>${report.compressedFiles.length}</strong>
				</div>
				<div>
					<span>Remote FastDL</span>
					<strong>${fastdlRemoteCount}</strong>
					<small>${escapeHtml(fastdlModeLabel(fastdlMode))}</small>
				</div>
				<div>
					<span>Destination conflicts</span>
					<strong class="${report.conflicts.length ? "danger-text" : ""}">${report.conflicts.length}</strong>
				</div>
			</div>
			${renderDeploymentWarnings(report)}
		</section>
	`;
}

function renderDeploymentWarnings(report: ValidationReport) {
	if (!config) {
		return "";
	}
	const warnings: string[] = [];
	if (config.storage.allowOverwrite && !config.storage.backupExisting && report.conflicts.length) {
		warnings.push("Local conflicts will be overwritten without backup.");
	}
	if (config.storage.serverInstallMode !== "local") {
		warnings.push("Remote game server overwrites are not backed up before upload.");
	}
	if (remoteMode(config) !== "none") {
		warnings.push("Remote FastDL overwrites are not backed up before upload.");
	}
	if (!warnings.length) {
		return "";
	}
	return `
		<div class="plan-warnings">
			${warnings.map((warning) => `<span>${escapeHtml(warning)}</span>`).join("")}
		</div>
	`;
}

function renderFileTable(files: FileSummary[]) {
	if (!files.length) {
		return `<p class="quiet">No files.</p>`;
	}
	return `
		<div class="file-table" role="table">
			${files.map((file) => `
				<div class="file-row" role="row">
					<span title="${escapeAttribute(file.path)}">${escapeHtml(file.path)}</span>
					<strong>${formatBytes(file.size)}</strong>
				</div>
			`).join("")}
		</div>
	`;
}

function renderCounts(items: CountItem[]) {
	if (!items.length) {
		return `<p class="quiet">No data.</p>`;
	}
	return `
		<div class="count-list">
			${items.map((item) => `
				<div>
					<span>${escapeHtml(item.name)}</span>
					<strong>${item.count}</strong>
				</div>
			`).join("")}
		</div>
	`;
}

function renderPathList(paths: string[]) {
	if (!paths.length) {
		return `<p class="quiet">No paths.</p>`;
	}
	return `
		<div class="path-list">
			${paths.map((path) => `<span title="${escapeAttribute(path)}">${escapeHtml(path)}</span>`).join("")}
		</div>
	`;
}

function bindEvents() {
	if (!config) {
		return;
	}
	const currentConfig = config;
	const serverRoot = getInput("server-root");
	const fastdlRoot = getInput("fastdl-root");
	const zipPath = getInput("zip-path");
	const contentType = getSelect("content-type");
	const formatGz = getInput("format-gz");
	const formatBz2 = getInput("format-bz2");
	const backupExisting = getInput("backup-existing");
	const allowOverwrite = getInput("allow-overwrite");
	const serverInstallMode = getSelect("server-install-mode");
	const serverFtpHost = getOptionalInput("server-ftp-host");
	const serverFtpPort = getOptionalInput("server-ftp-port");
	const serverFtpUsername = getOptionalInput("server-ftp-username");
	const serverFtpPassword = getOptionalInput("server-ftp-password");
	const serverFtpRemoteRoot = getOptionalInput("server-ftp-remote-root");
	const serverSftpHost = getOptionalInput("server-sftp-host");
	const serverSftpPort = getOptionalInput("server-sftp-port");
	const serverSftpUsername = getOptionalInput("server-sftp-username");
	const serverSftpPassword = getOptionalInput("server-sftp-password");
	const serverSftpPrivateKey = getOptionalInput("server-sftp-private-key");
	const serverSftpKeyPassphrase = getOptionalInput("server-sftp-key-passphrase");
	const serverSftpRemoteRoot = getOptionalInput("server-sftp-remote-root");
	const remoteModeSelect = getSelect("remote-mode");
	const ftpHost = getOptionalInput("ftp-host");
	const ftpPort = getOptionalInput("ftp-port");
	const ftpUsername = getOptionalInput("ftp-username");
	const ftpPassword = getOptionalInput("ftp-password");
	const ftpRemoteRoot = getOptionalInput("ftp-remote-root");
	const sftpHost = getOptionalInput("sftp-host");
	const sftpPort = getOptionalInput("sftp-port");
	const sftpUsername = getOptionalInput("sftp-username");
	const sftpPassword = getOptionalInput("sftp-password");
	const sftpPrivateKey = getOptionalInput("sftp-private-key");
	const sftpKeyPassphrase = getOptionalInput("sftp-key-passphrase");
	const sftpRemoteRoot = getOptionalInput("sftp-remote-root");

	serverRoot.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverRoot = serverRoot.value.trim();
	}));
	fastdlRoot.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.fastdlRoot = fastdlRoot.value.trim() || null;
	}));
	zipPath.addEventListener("input", () => {
		selectedZipPath = zipPath.value.trim();
	});
	contentType.addEventListener("change", () => {
		selectedContentType = contentType.value;
		validationReport = null;
		render();
	});
	formatGz.addEventListener("change", syncFormats);
	formatBz2.addEventListener("change", syncFormats);
	backupExisting.addEventListener("change", () => updateConfig((draft) => {
		draft.storage.backupExisting = backupExisting.checked;
	}));
	allowOverwrite.addEventListener("change", () => updateConfig((draft) => {
		draft.storage.allowOverwrite = allowOverwrite.checked;
	}));
	serverInstallMode.addEventListener("change", () => {
		updateConfig((draft) => {
			draft.storage.serverInstallMode = serverInstallMode.value as AppConfig["storage"]["serverInstallMode"];
			draft.storage.serverFtp.enabled = serverInstallMode.value === "ftp";
			draft.storage.serverSftp.enabled = serverInstallMode.value === "sftp";
		});
		render();
	});
	serverFtpHost?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverFtp.host = serverFtpHost.value.trim() || null;
	}));
	serverFtpPort?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverFtp.port = Number.parseInt(serverFtpPort.value, 10) || 21;
	}));
	serverFtpUsername?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverFtp.username = serverFtpUsername.value.trim() || null;
	}));
	serverFtpPassword?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverFtp.password = serverFtpPassword.value || null;
	}));
	serverFtpRemoteRoot?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverFtp.remoteFastdlRoot = serverFtpRemoteRoot.value.trim() || null;
	}));
	serverSftpHost?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverSftp.host = serverSftpHost.value.trim() || null;
		draft.storage.serverSftp.trustedHostFingerprint = null;
	}));
	serverSftpPort?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverSftp.port = Number.parseInt(serverSftpPort.value, 10) || 22;
		draft.storage.serverSftp.trustedHostFingerprint = null;
	}));
	serverSftpUsername?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverSftp.username = serverSftpUsername.value.trim() || null;
	}));
	serverSftpPassword?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverSftp.password = serverSftpPassword.value || null;
	}));
	serverSftpPrivateKey?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverSftp.privateKeyPath = serverSftpPrivateKey.value.trim() || null;
	}));
	serverSftpKeyPassphrase?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverSftp.privateKeyPassphrase = serverSftpKeyPassphrase.value || null;
	}));
	serverSftpRemoteRoot?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.serverSftp.remoteFastdlRoot = serverSftpRemoteRoot.value.trim() || null;
	}));
	remoteModeSelect.addEventListener("change", () => {
		updateConfig((draft) => {
			draft.storage.ftp.enabled = remoteModeSelect.value === "ftp";
			draft.storage.sftp.enabled = remoteModeSelect.value === "sftp";
		});
		render();
	});
	ftpHost?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.ftp.host = ftpHost.value.trim() || null;
	}));
	ftpPort?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.ftp.port = Number.parseInt(ftpPort.value, 10) || 21;
	}));
	ftpUsername?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.ftp.username = ftpUsername.value.trim() || null;
	}));
	ftpPassword?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.ftp.password = ftpPassword.value || null;
	}));
	ftpRemoteRoot?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.ftp.remoteFastdlRoot = ftpRemoteRoot.value.trim() || null;
	}));
	sftpHost?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.sftp.host = sftpHost.value.trim() || null;
		draft.storage.sftp.trustedHostFingerprint = null;
	}));
	sftpPort?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.sftp.port = Number.parseInt(sftpPort.value, 10) || 22;
		draft.storage.sftp.trustedHostFingerprint = null;
	}));
	sftpUsername?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.sftp.username = sftpUsername.value.trim() || null;
	}));
	sftpPassword?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.sftp.password = sftpPassword.value || null;
	}));
	sftpPrivateKey?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.sftp.privateKeyPath = sftpPrivateKey.value.trim() || null;
	}));
	sftpKeyPassphrase?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.sftp.privateKeyPassphrase = sftpKeyPassphrase.value || null;
	}));
	sftpRemoteRoot?.addEventListener("input", () => updateConfig((draft) => {
		draft.storage.sftp.remoteFastdlRoot = sftpRemoteRoot.value.trim() || null;
	}));

	getButton("pick-server-root").addEventListener("click", () => void pickDirectory("server"));
	getButton("pick-fastdl-root").addEventListener("click", () => void pickDirectory("fastdl"));
	getOptionalButton("pick-server-sftp-key")?.addEventListener("click", () => void pickServerSftpKey());
	getOptionalButton("pick-sftp-key")?.addEventListener("click", () => void pickSftpKey());
	getOptionalButton("test-server-ftp")?.addEventListener("click", () => void testFtpConnection("Game Server FTP", currentConfig.storage.serverFtp));
	getOptionalButton("test-server-sftp")?.addEventListener("click", () => void testSftpConnection(
		"Game Server SFTP",
		currentConfig.storage.serverSftp,
		(draft, fingerprint) => {
			draft.storage.serverSftp.trustedHostFingerprint = fingerprint;
		},
	));
	getOptionalButton("test-fastdl-ftp")?.addEventListener("click", () => void testFtpConnection("FastDL FTP", currentConfig.storage.ftp));
	getOptionalButton("test-fastdl-sftp")?.addEventListener("click", () => void testSftpConnection(
		"FastDL SFTP",
		currentConfig.storage.sftp,
		(draft, fingerprint) => {
			draft.storage.sftp.trustedHostFingerprint = fingerprint;
		},
	));
	getButton("pick-zip").addEventListener("click", () => void pickZip());
	getButton("validate").addEventListener("click", () => void validatePackage());
	getButton("install").addEventListener("click", () => void installPackage());
	getButton("refresh-logs").addEventListener("click", () => void refreshLogs());
	getButton("open-logs-folder").addEventListener("click", () => void openLogsFolder());
	for (const button of document.querySelectorAll<HTMLButtonElement>(".rollback-button")) {
		button.addEventListener("click", () => void rollbackUpload(button.dataset.uploadId ?? ""));
	}
}

function syncFormats() {
	const formats: CompressedFormat[] = [];
	if (getInput("format-gz").checked) {
		formats.push("gz");
	}
	if (getInput("format-bz2").checked) {
		formats.push("bz2");
	}
	updateConfig((draft) => {
		draft.storage.compressedFormats = formats;
	});
}

async function pickDirectory(kind: "server" | "fastdl") {
	const selected = await open({
		directory: true,
		multiple: false,
	});
	if (typeof selected !== "string") {
		return;
	}
	updateConfig((draft) => {
		if (kind === "server") {
			draft.storage.serverRoot = selected;
		} else {
			draft.storage.fastdlRoot = selected;
		}
	});
	await saveConfigNow();
	validationReport = null;
	render();
}

async function pickZip() {
	const selected = await open({
		directory: false,
		multiple: false,
		filters: [
			{
				name: "Package archive",
				extensions: ["zip", "tar", "gz", "tgz", "bz2", "tbz2"],
			},
		],
	});
	if (typeof selected !== "string") {
		return;
	}
	selectedZipPath = selected;
	validationReport = null;
	render();
}

async function pickSftpKey() {
	const selected = await open({
		directory: false,
		multiple: false,
	});
	if (typeof selected !== "string") {
		return;
	}
	updateConfig((draft) => {
		draft.storage.sftp.privateKeyPath = selected;
	});
	await saveConfigNow();
	render();
}

async function pickServerSftpKey() {
	const selected = await open({
		directory: false,
		multiple: false,
	});
	if (typeof selected !== "string") {
		return;
	}
	updateConfig((draft) => {
		draft.storage.serverSftp.privateKeyPath = selected;
	});
	await saveConfigNow();
	render();
}

async function testFtpConnection(label: string, remoteConfig: AppConfig["storage"]["ftp"]) {
	if (busy) {
		return;
	}
	await saveConfigNow();
	errorMessage = "";
	statusMessage = `Testing ${label}`;
	busy = true;
	render();
	try {
		await invoke("test_ftp_connection", {
			config: remoteConfig,
		});
		statusMessage = `${label} connection OK`;
	} catch (error) {
		errorMessage = formatError(error);
		statusMessage = "Connection failed";
	} finally {
		busy = false;
		render();
	}
}

async function testSftpConnection(
	label: string,
	remoteConfig: AppConfig["storage"]["sftp"],
	trustFingerprint: (draft: AppConfig, fingerprint: string) => void,
) {
	if (busy) {
		return;
	}
	await saveConfigNow();
	errorMessage = "";
	statusMessage = `Inspecting ${label} host`;
	busy = true;
	render();
	try {
		const trustedConfig = await ensureTrustedSftpHost(label, remoteConfig, trustFingerprint);
		if (!trustedConfig) {
			statusMessage = `${label} trust not confirmed`;
			return;
		}
		statusMessage = `Testing ${label}`;
		render();
		await invoke("test_sftp_connection", {
			config: trustedConfig,
		});
		statusMessage = `${label} connection OK`;
	} catch (error) {
		errorMessage = formatError(error);
		statusMessage = "Connection failed";
	} finally {
		busy = false;
		render();
	}
}

async function ensureTrustedSftpHost(
	label: string,
	remoteConfig: AppConfig["storage"]["sftp"],
	trustFingerprint: (draft: AppConfig, fingerprint: string) => void,
) {
	const fingerprint = await invoke<SftpHostFingerprint>("inspect_sftp_host", {
		config: remoteConfig,
	});
	const trusted = remoteConfig.trustedHostFingerprint;
	if (!trusted) {
		const confirmed = window.confirm(
			`Trust this ${label} host?\n\nHost: ${fingerprint.host}:${fingerprint.port}\nHost key: ${fingerprint.hostKeyType}\nFingerprint: ${fingerprint.sha256}\n\nOnly continue if this matches your hosting panel or FileZilla server fingerprint.`,
		);
		if (!confirmed) {
			return null;
		}
		updateConfig((draft) => trustFingerprint(draft, fingerprint.sha256));
		remoteConfig.trustedHostFingerprint = fingerprint.sha256;
		await saveConfigNow();
		return remoteConfig;
	}
	if (trusted !== fingerprint.sha256) {
		const confirmed = window.confirm(
			`${label} host fingerprint changed.\n\nExpected: ${trusted}\nCurrent: ${fingerprint.sha256}\nHost key: ${fingerprint.hostKeyType}\n\nThis can be a legitimate server replacement or a man-in-the-middle attack. Replace the trusted fingerprint and continue?`,
		);
		if (!confirmed) {
			return null;
		}
		updateConfig((draft) => trustFingerprint(draft, fingerprint.sha256));
		remoteConfig.trustedHostFingerprint = fingerprint.sha256;
		await saveConfigNow();
		return remoteConfig;
	}
	return remoteConfig;
}

async function validatePackage() {
	if (!config || busy) {
		return;
	}
	await saveConfigNow();
	errorMessage = "";
	statusMessage = "Validating package";
	busy = true;
	render();

	try {
		validationReport = await invoke<ValidationReport>("validate_package", {
			config,
			zipPath: selectedZipPath,
			contentType: selectedContentType,
		});
		installReport = null;
		statusMessage = validationReport.conflicts.length ? "Validated with conflicts" : "Validated";
	} catch (error) {
		validationReport = null;
		errorMessage = formatError(error);
		statusMessage = "Error";
	} finally {
		busy = false;
		render();
	}
}

async function installPackage() {
	if (!config || busy || !validationReport) {
		return;
	}
	await saveConfigNow();
	if (validationReport.conflicts.length) {
		const confirmed = window.confirm(
			`Install will replace ${validationReport.conflicts.length} existing destination file(s). Continue?`,
		);
		if (!confirmed) {
			return;
		}
	}
	if (config.storage.allowOverwrite && !config.storage.backupExisting && validationReport.conflicts.length) {
		const confirmed = window.confirm(
			"Back up before replacing is disabled.\n\nExisting local files that conflict with this package will be overwritten without a restore copy. Continue?",
		);
		if (!confirmed) {
			return;
		}
	}
	errorMessage = "";
	statusMessage = "Installing package";
	busy = true;
	render();

	try {
		progressEvents = [];
		installReport = await invoke<InstallReport>("install_package", {
			config,
			zipPath: selectedZipPath,
			contentType: selectedContentType,
		});
		validationReport = installReport.validation;
		await loadUploads();
		await loadLogSnapshot();
		statusMessage = "Installed";
	} catch (error) {
		errorMessage = formatError(error);
		statusMessage = "Error";
	} finally {
		busy = false;
		render();
	}
}

async function rollbackUpload(uploadId: string) {
	if (!config || busy || !uploadId) {
		return;
	}
	await saveConfigNow();
	const upload = uploadHistory.find((item) => item.uploadId === uploadId);
	const installedCount = upload?.installedFiles.length ?? 0;
	const compressedCount = upload?.compressedFiles.length ?? 0;
	const remoteCount = (upload?.serverPublishedFiles?.length ?? 0) + (upload?.ftpPublishedFiles.length ?? 0) + (upload?.sftpPublishedFiles.length ?? 0);
	const confirmed = window.confirm(
		`Roll back upload ${uploadId}?\n\nThis will remove ${installedCount + compressedCount} installed/generated file(s), remove ${remoteCount} remote published file(s), and restore any backups recorded in the manifest.`,
	);
	if (!confirmed) {
		return;
	}
	errorMessage = "";
	statusMessage = "Rolling back upload";
	busy = true;
	render();

	try {
		await invoke("rollback_upload", {
			config,
			uploadId,
			force: true,
		});
		await loadUploads();
		await loadLogSnapshot();
		statusMessage = "Rolled back";
	} catch (error) {
		errorMessage = formatError(error);
		statusMessage = "Error";
	} finally {
		busy = false;
		render();
	}
}

async function loadUploads() {
	if (!config || !config.storage.serverRoot) {
		uploadHistory = [];
		return;
	}
	try {
		uploadHistory = await invoke<UploadManifest[]>("list_uploads", {
			config,
		});
	} catch {
		uploadHistory = [];
	}
}

async function loadLogSnapshot() {
	if (!config || !config.storage.serverRoot) {
		logSnapshot = null;
		return;
	}
	try {
		logSnapshot = await invoke<LogSnapshot>("get_log_snapshot", {
			config,
		});
	} catch {
		logSnapshot = null;
	}
}

async function refreshLogs() {
	if (!config || busy || logViewerLoading) {
		return;
	}
	await saveConfigNow();
	errorMessage = "";
	statusMessage = "Refreshing logs";
	logViewerLoading = true;
	render();
	try {
		await loadUploads();
		await loadLogSnapshot();
		statusMessage = "Logs refreshed";
	} catch (error) {
		errorMessage = formatError(error);
		statusMessage = "Could not refresh logs";
	} finally {
		logViewerLoading = false;
		render();
	}
}

async function openLogsFolder() {
	if (!config || busy || logViewerLoading) {
		return;
	}
	await saveConfigNow();
	errorMessage = "";
	statusMessage = "Opening logs folder";
	logViewerLoading = true;
	render();
	try {
		await invoke("open_logs_folder", {
			config,
		});
		await loadLogSnapshot();
		statusMessage = "Logs folder opened";
	} catch (error) {
		errorMessage = formatError(error);
		statusMessage = "Could not open logs folder";
	} finally {
		logViewerLoading = false;
		render();
	}
}

function updateConfig(mutator: (draft: AppConfig) => void) {
	if (!config) {
		return;
	}
	mutator(config);
	configVersion += 1;
	configDirty = true;
	scheduleConfigSave();
}

function scheduleConfigSave() {
	if (configSaveTimer !== null) {
		window.clearTimeout(configSaveTimer);
	}
	configSaveTimer = window.setTimeout(() => {
		configSaveTimer = null;
		void saveConfig();
	}, 50);
}

async function saveConfig() {
	await saveConfigNow();
}

async function saveConfigNow() {
	if (!config) {
		return true;
	}
	if (configSaveTimer !== null) {
		window.clearTimeout(configSaveTimer);
		configSaveTimer = null;
	}
	if (!configDirty) {
		return true;
	}
	if (configSaveInFlight) {
		configSaveQueued = true;
		return true;
	}
	const versionToSave = configVersion;
	configSaveInFlight = true;
	let saved = false;
	try {
		await invoke("save_config", {
			config,
		});
		saved = true;
		savedConfigVersion = Math.max(savedConfigVersion, versionToSave);
		configDirty = configVersion > savedConfigVersion;
		return true;
	} catch (error) {
		errorMessage = formatError(error);
		statusMessage = "Could not save configuration";
		render();
		return false;
	} finally {
		configSaveInFlight = false;
		if (saved && (configSaveQueued || configDirty)) {
			configSaveQueued = false;
			void saveConfigNow();
		}
	}
}

function hasBlockingConflicts() {
	return Boolean(
		config
			&& validationReport
			&& validationReport.conflicts.length > 0
			&& !config.storage.allowOverwrite,
	);
}

function remoteMode(currentConfig: AppConfig) {
	if (currentConfig.storage.ftp.enabled) {
		return "ftp";
	}
	if (currentConfig.storage.sftp.enabled) {
		return "sftp";
	}
	return "none";
}

function serverModeLabel(mode: AppConfig["storage"]["serverInstallMode"]) {
	if (mode === "ftp") {
		return "Local + FTP";
	}
	if (mode === "sftp") {
		return "Local + SFTP";
	}
	return "Local only";
}

function fastdlModeLabel(mode: "none" | "ftp" | "sftp") {
	if (mode === "ftp") {
		return "FTP";
	}
	if (mode === "sftp") {
		return "SFTP";
	}
	return "None";
}

function getInput(id: string) {
	const element = document.getElementById(id);
	if (!(element instanceof HTMLInputElement)) {
		throw new Error(`Input ${id} was not found`);
	}
	return element;
}

function getOptionalInput(id: string) {
	const element = document.getElementById(id);
	if (!element) {
		return null;
	}
	if (!(element instanceof HTMLInputElement)) {
		throw new Error(`Input ${id} was not found`);
	}
	return element;
}

function getSelect(id: string) {
	const element = document.getElementById(id);
	if (!(element instanceof HTMLSelectElement)) {
		throw new Error(`Select ${id} was not found`);
	}
	return element;
}

function getButton(id: string) {
	const element = document.getElementById(id);
	if (!(element instanceof HTMLButtonElement)) {
		throw new Error(`Button ${id} was not found`);
	}
	return element;
}

function getOptionalButton(id: string) {
	const element = document.getElementById(id);
	if (!element) {
		return null;
	}
	if (!(element instanceof HTMLButtonElement)) {
		throw new Error(`Button ${id} was not found`);
	}
	return element;
}

function formatBytes(bytes: number) {
	const units = ["B", "KB", "MB", "GB"];
	let value = bytes;
	let unit = units[0];
	for (let index = 1; index < units.length && value >= 1024; index += 1) {
		value /= 1024;
		unit = units[index];
	}
	return `${value.toFixed(value >= 10 || unit === "B" ? 0 : 1)} ${unit}`;
}

function formatError(error: unknown) {
	return error instanceof Error ? error.message : String(error);
}

function escapeHtml(value: string) {
	return value
		.replaceAll("&", "&amp;")
		.replaceAll("<", "&lt;")
		.replaceAll(">", "&gt;")
		.replaceAll('"', "&quot;")
		.replaceAll("'", "&#039;");
}

function escapeAttribute(value: string) {
	return escapeHtml(value).replaceAll("`", "&#096;");
}
