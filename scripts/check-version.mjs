import { readFileSync } from "node:fs";

const packageVersion = JSON.parse(readFileSync("package.json", "utf8")).version;
const tauriVersion = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).version;
const cargoManifest = readFileSync("src-tauri/Cargo.toml", "utf8");
const changelog = readFileSync("CHANGELOG.md", "utf8");
const cargoVersion = cargoManifest.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const versions = { packageVersion, tauriVersion, cargoVersion };

if (new Set(Object.values(versions)).size !== 1) {
  throw new Error(`Release versions are out of sync: ${JSON.stringify(versions)}`);
}

const escapedVersion = packageVersion.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
if (!new RegExp(`^## \\[${escapedVersion}\\] - \\d{4}-\\d{2}-\\d{2}$`, "m").test(changelog)) {
  throw new Error(`CHANGELOG.md is missing a dated ${packageVersion} release heading.`);
}

console.log(`Release version ${packageVersion} is synchronized.`);
