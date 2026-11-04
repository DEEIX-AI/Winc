// The release version is `version` in the [workspace.package] section of Cargo.toml. Run directly,
// this script prints it.
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

export function workspaceVersion(directory = root) {
  const manifest = readFileSync(join(directory, "Cargo.toml"), "utf8");
  const section = /^\[workspace\.package\]\r?\n([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(manifest)?.[1];
  const version = section && /^version\s*=\s*"([^"]+)"/m.exec(section)?.[1];
  if (!version) throw new Error("Cargo.toml has no version in [workspace.package]");
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`Cargo.toml version is not a semantic version: ${version}`);
  }
  return version;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { console.log(workspaceVersion()); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
