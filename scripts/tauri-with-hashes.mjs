import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

const scriptPath = fileURLToPath(import.meta.url);
const projectRoot = path.resolve(path.dirname(scriptPath), "..");
const args = process.argv.slice(2);

const tauriBin = path.join(projectRoot, "node_modules", "@tauri-apps", "cli", "tauri.js");

const exitCode = await runTauri();
if (exitCode !== 0) {
	process.exit(exitCode);
}

if (args[0] === "build") {
	await writeReleaseHashes();
}

function runTauri() {
	return new Promise((resolve) => {
		const child = spawn(process.execPath, [tauriBin, ...args], {
			cwd: projectRoot,
			stdio: "inherit",
		});
		child.on("exit", (code) => resolve(code ?? 1));
		child.on("error", (error) => {
			console.error(`Could not start Tauri CLI: ${error.message}`);
			resolve(1);
		});
	});
}

async function writeReleaseHashes() {
	const bundleRoot = path.join(projectRoot, "src-tauri", "target", "release", "bundle");
	const artifacts = await findArtifacts(bundleRoot);
	if (!artifacts.length) {
		console.warn("No release artifacts found for SHA256SUMS.txt.");
		return;
	}

	const lines = [];
	for (const artifact of artifacts) {
		const hash = await sha256File(artifact);
		const relative = normalizePath(path.relative(bundleRoot, artifact));
		lines.push(`${hash}  ${relative}`);
	}
	lines.sort((left, right) => left.localeCompare(right));

	const text = `${lines.join("\n")}\n`;
	await writeFile(path.join(bundleRoot, "SHA256SUMS.txt"), text);
	await writePerDirectoryHashes(bundleRoot, artifacts);
	console.log(`Wrote SHA256SUMS.txt for ${artifacts.length} release artifact(s).`);
}

async function findArtifacts(bundleRoot) {
	if (!existsSync(bundleRoot)) {
		return [];
	}
	const artifacts = [];
	await collectArtifacts(bundleRoot, artifacts);
	return artifacts;
}

async function collectArtifacts(directory, artifacts) {
	for (const entry of await readdir(directory, { withFileTypes: true })) {
		const current = path.join(directory, entry.name);
		if (entry.isDirectory()) {
			await collectArtifacts(current, artifacts);
			continue;
		}
		if (entry.isFile() && isReleaseArtifact(entry.name)) {
			artifacts.push(current);
		}
	}
}

function isReleaseArtifact(fileName) {
	const lower = fileName.toLowerCase();
	return [
		".msi",
		".exe",
		".deb",
		".rpm",
		".appimage",
		".dmg",
	].some((extension) => lower.endsWith(extension));
}

async function writePerDirectoryHashes(bundleRoot, artifacts) {
	const byDirectory = new Map();
	for (const artifact of artifacts) {
		const directory = path.dirname(artifact);
		const list = byDirectory.get(directory) ?? [];
		list.push(artifact);
		byDirectory.set(directory, list);
	}

	for (const [directory, files] of byDirectory) {
		const lines = [];
		for (const file of files) {
			const hash = await sha256File(file);
			lines.push(`${hash}  ${normalizePath(path.relative(directory, file))}`);
		}
		lines.sort((left, right) => left.localeCompare(right));
		await mkdir(directory, { recursive: true });
		await writeFile(path.join(directory, "SHA256SUMS.txt"), `${lines.join("\n")}\n`);
	}
}

async function sha256File(file) {
	const hash = createHash("sha256");
	const buffer = await readFile(file);
	hash.update(buffer);
	return hash.digest("hex");
}

function normalizePath(value) {
	return value.split(path.sep).join("/");
}
