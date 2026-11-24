#!/usr/bin/env node
// Keeps the Cargo workspace version in step with VERSION, the release line the tag workflow reads
// (`Version file` in docs/agents/releases.md). This script belongs to this repository; it is not
// one of the canonical jwh3times/skills scripts.
//
//   node scripts/check-cargo-version.mjs                  # fails when the two differ
//   node scripts/check-cargo-version.mjs --set <x.y.z>    # writes <x.y.z> to both
//
// `scripts/next-version.mjs` reads a non-JSON version file as one bare version, so it cannot read
// Cargo.toml; VERSION holds the line, and `[workspace.package].version` in Cargo.toml, which every
// crate inherits and `sol --version` reports, must name the same one. CI's `agents` job runs the
// check. `--set` is the `Bump command` half that edits files; `cargo update --workspace`
// then refreshes the workspace crates' versions in Cargo.lock. The release workflow runs `--set`
// with the tag's full version before it builds, so a released binary reports its own tag.

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

// The span of the `version = "..."` value inside `[workspace.package]`, or null when the section
// or its version key is absent. Offsets index into `toml` as given, so a replacement keeps every
// other byte (line endings included) as it was.
export function findWorkspaceVersion(toml) {
  const header = /^\[workspace\.package\][ \t]*$/m.exec(toml);
  if (!header) return null;
  const start = header.index + header[0].length;
  const next = /^\[/m.exec(toml.slice(start));
  const body = toml.slice(start, next ? start + next.index : undefined);
  const key = /^version[ \t]*=[ \t]*"([^"\r\n]*)"/m.exec(body);
  if (!key) return null;
  const valueStart = start + key.index + key[0].indexOf('"') + 1;
  return { value: key[1], start: valueStart, end: valueStart + key[1].length };
}

export function setWorkspaceVersion(toml, version) {
  const found = findWorkspaceVersion(toml);
  if (!found) throw new Error("Cargo.toml has no version in [workspace.package].");
  return toml.slice(0, found.start) + version + toml.slice(found.end);
}

export function check(root = repositoryRoot) {
  const declared = readFileSync(path.join(root, "VERSION"), "utf8").trim();
  const found = findWorkspaceVersion(readFileSync(path.join(root, "Cargo.toml"), "utf8"));
  if (!found) return { ok: false, message: "Cargo.toml has no version in [workspace.package]." };
  if (found.value !== declared) {
    return {
      ok: false,
      message:
        `Cargo.toml's workspace version is ${found.value}, but VERSION is ${declared}. ` +
        `Run: node scripts/check-cargo-version.mjs --set ${declared} && cargo update --workspace`,
    };
  }
  return { ok: true, message: `Cargo.toml's workspace version matches VERSION (${declared}).` };
}

export function set(version, root = repositoryRoot) {
  if (!SEMVER.test(version)) throw new Error(`"${version}" is not a plain major.minor.build SemVer.`);
  const cargo = path.join(root, "Cargo.toml");
  writeFileSync(cargo, setWorkspaceVersion(readFileSync(cargo, "utf8"), version));
  writeFileSync(path.join(root, "VERSION"), `${version}\n`);
}

export function main(argv, root = repositoryRoot) {
  if (argv[0] === "--set") {
    if (argv.length !== 2) throw new Error("Usage: node scripts/check-cargo-version.mjs --set <x.y.z>");
    set(argv[1], root);
    console.log(`VERSION and Cargo.toml's workspace version are now ${argv[1]}.`);
    return 0;
  }
  if (argv.length > 0) throw new Error(`Unknown arguments: ${argv.join(" ")}`);
  const result = check(root);
  (result.ok ? console.log : console.error)(result.message);
  return result.ok ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
