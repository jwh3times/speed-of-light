import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, test } from "node:test";

import {
  check,
  findWorkspaceVersion,
  main,
  set,
  setWorkspaceVersion,
} from "./check-cargo-version.mjs";

const CARGO = `[workspace]
resolver = "3"
members = ["crates/a"]

[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
a = { path = "crates/a", version = "9.9.9" }
`;

describe("findWorkspaceVersion", () => {
  test("reads the version under [workspace.package]", () => {
    assert.equal(findWorkspaceVersion(CARGO)?.value, "0.1.0");
  });

  test("ignores a version key in another section", () => {
    const toml = `[package]\nversion = "5.0.0"\n\n[workspace.package]\nedition = "2024"\n`;
    assert.equal(findWorkspaceVersion(toml), null);
  });

  test("ignores a version key in a section after [workspace.package]", () => {
    const toml = `[workspace.package]\nedition = "2024"\n\n[package]\nversion = "5.0.0"\n`;
    assert.equal(findWorkspaceVersion(toml), null);
  });

  test("reads version, not rust-version, whichever comes first", () => {
    const toml = `[workspace.package]\nrust-version = "1.88"\nversion = "0.1.0"\n`;
    assert.equal(findWorkspaceVersion(toml)?.value, "0.1.0");
  });

  test("is null without a [workspace.package] section", () => {
    assert.equal(findWorkspaceVersion(`[package]\nversion = "1.0.0"\n`), null);
  });

  test("reads a CRLF file", () => {
    assert.equal(findWorkspaceVersion(CARGO.replaceAll("\n", "\r\n"))?.value, "0.1.0");
  });
});

describe("setWorkspaceVersion", () => {
  test("changes only the workspace version", () => {
    const updated = setWorkspaceVersion(CARGO, "0.2.0");
    assert.equal(updated, CARGO.replace('version = "0.1.0"', 'version = "0.2.0"'));
    assert.match(updated, /version = "9\.9\.9"/);
  });

  test("keeps CRLF line endings", () => {
    const crlf = CARGO.replaceAll("\n", "\r\n");
    assert.equal(setWorkspaceVersion(crlf, "1.0.0"), crlf.replace("0.1.0", "1.0.0"));
  });

  test("throws when there is no workspace version", () => {
    assert.throws(() => setWorkspaceVersion("[workspace]\n", "1.0.0"), /no version/);
  });
});

describe("check and set", () => {
  let root;
  beforeEach(() => {
    root = mkdtempSync(path.join(tmpdir(), "cargo-version-"));
    writeFileSync(path.join(root, "Cargo.toml"), CARGO);
  });
  afterEach(() => rmSync(root, { recursive: true, force: true }));

  test("passes when VERSION matches", () => {
    writeFileSync(path.join(root, "VERSION"), "0.1.0\n");
    assert.equal(check(root).ok, true);
  });

  test("fails and names the fix when VERSION differs", () => {
    writeFileSync(path.join(root, "VERSION"), "0.2.0\n");
    const result = check(root);
    assert.equal(result.ok, false);
    assert.match(result.message, /is 0\.1\.0, but VERSION is 0\.2\.0/);
    assert.match(result.message, /--set 0\.2\.0/);
  });

  test("set writes both files, and the check then passes", () => {
    writeFileSync(path.join(root, "VERSION"), "0.1.0\n");
    set("0.3.0", root);
    assert.equal(readFileSync(path.join(root, "VERSION"), "utf8"), "0.3.0\n");
    assert.equal(findWorkspaceVersion(readFileSync(path.join(root, "Cargo.toml"), "utf8"))?.value, "0.3.0");
    assert.equal(check(root).ok, true);
  });

  test("set twice with the same version is a no-op the second time", () => {
    set("0.3.0", root);
    const first = readFileSync(path.join(root, "Cargo.toml"), "utf8");
    set("0.3.0", root);
    assert.equal(readFileSync(path.join(root, "Cargo.toml"), "utf8"), first);
  });

  test("set rejects a version that is not plain SemVer", () => {
    assert.throws(() => set("v0.3.0", root), /not a plain/);
    assert.throws(() => set("0.3", root), /not a plain/);
    assert.throws(() => set("0.3.0-rc.1", root), /not a plain/);
    assert.throws(() => set("0.3.0.1", root), /not a plain/);
  });

  test("check fails when Cargo.toml has no workspace version", () => {
    writeFileSync(path.join(root, "Cargo.toml"), `[package]\nname = "x"\nversion = "0.1.0"\n`);
    writeFileSync(path.join(root, "VERSION"), "0.1.0\n");
    const result = check(root);
    assert.equal(result.ok, false);
    assert.match(result.message, /no version/);
  });

  test("main returns 1 on a mismatch and 0 once set", () => {
    writeFileSync(path.join(root, "VERSION"), "0.2.0\n");
    const errors = [];
    const original = console.error;
    console.error = (line) => errors.push(line);
    try {
      assert.equal(main([], root), 1);
    } finally {
      console.error = original;
    }
    assert.equal(errors.length, 1);
    const logs = [];
    const log = console.log;
    console.log = (line) => logs.push(line);
    try {
      assert.equal(main(["--set", "0.2.0"], root), 0);
      assert.equal(main([], root), 0);
    } finally {
      console.log = log;
    }
  });
});
