#!/usr/bin/env node
/**
 * Local extension installer — bypasses `code --install-extension` which is
 * unreliable from VS Code's integrated terminal on Windows.
 *
 * Copies compiled extension files directly into the VS Code extensions
 * directory, cleans the `.obsolete` flag, and registers in extensions.json.
 *
 * Usage: node scripts/install-local.js
 */

const fs = require("fs");
const path = require("path");
const os = require("os");

// Derive everything from package.json — zero hardcoding
const pkgPath = path.join(__dirname, "..", "package.json");
const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
const extId = `${pkg.publisher}.${pkg.name}`;
const extDir = `${extId}-${pkg.version}`;

// VS Code extensions directory (cross-platform)
const vscodeExtDir =
	process.env.VSCODE_EXTENSIONS ||
	path.join(os.homedir(), ".vscode", "extensions");

const targetDir = path.join(vscodeExtDir, extDir);
const obsoletePath = path.join(vscodeExtDir, ".obsolete");
const registryPath = path.join(vscodeExtDir, "extensions.json");

// Files/dirs to copy from source
const srcRoot = path.join(__dirname, "..");
const toCopy = [
	"out",
	"syntaxes",
	"node_modules",
	"language-configuration.json",
	"package.json",
	"readme.md",
];

function copyRecursive(src, dest) {
	if (!fs.existsSync(src)) return;
	const stat = fs.statSync(src);
	if (stat.isDirectory()) {
		fs.mkdirSync(dest, { recursive: true });
		for (const child of fs.readdirSync(src)) {
			copyRecursive(path.join(src, child), path.join(dest, child));
		}
	} else {
		fs.copyFileSync(src, dest);
	}
}

// 1. Remove old extension directory if it exists
if (fs.existsSync(targetDir)) {
	fs.rmSync(targetDir, { recursive: true, force: true });
}
fs.mkdirSync(targetDir, { recursive: true });

// 2. Copy files
let fileCount = 0;
for (const item of toCopy) {
	const src = path.join(srcRoot, item);
	const dest = path.join(targetDir, item);
	if (fs.existsSync(src)) {
		copyRecursive(src, dest);
		fileCount++;
	}
}
console.log(`  Copied ${fileCount} items to ${targetDir}`);

// 3. Remove from .obsolete if present
if (fs.existsSync(obsoletePath)) {
	try {
		const obsolete = JSON.parse(fs.readFileSync(obsoletePath, "utf8"));
		if (obsolete[extDir]) {
			delete obsolete[extDir];
			fs.writeFileSync(obsoletePath, JSON.stringify(obsolete));
			console.log(`  Removed ${extDir} from .obsolete`);
		}
	} catch {
		// If .obsolete is malformed, leave it alone
	}
}

// 4. Register in extensions.json if not present
if (fs.existsSync(registryPath)) {
	try {
		const registry = JSON.parse(fs.readFileSync(registryPath, "utf8"));
		const idx = registry.findIndex(
			(e) => e.identifier && e.identifier.id === extId,
		);
		const locationPath = targetDir
			.replace(/\\/g, "/")
			.replace(/^([A-Z]):/, (_, d) => `/${d.toLowerCase()}:`);
		const entry = {
			identifier: { id: extId },
			version: pkg.version,
			location: { $mid: 1, path: locationPath, scheme: "file" },
			relativeLocation: extDir,
			metadata: {
				targetPlatform: "undefined",
				updated: false,
				isPreReleaseVersion: false,
				installedTimestamp: Date.now(),
				preRelease: false,
				isApplicationScoped: false,
			},
		};
		if (idx >= 0) {
			registry[idx] = entry;
		} else {
			registry.push(entry);
		}
		fs.writeFileSync(registryPath, JSON.stringify(registry, null, 2));
		console.log(`  Registered in extensions.json`);
	} catch {
		// If registry is malformed, continue — VS Code will regenerate
	}
}

console.log(`\n  Extension ${extId} v${pkg.version} installed successfully.`);
console.log(
	`  Reload VS Code (Ctrl+Shift+P → "Developer: Reload Window") to activate.\n`,
);
