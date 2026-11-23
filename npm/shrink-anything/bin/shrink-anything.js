#!/usr/bin/env node
// Runs the prebuilt shrink-anything binary for this platform (installed as an optional dependency).
"use strict";
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");

const pkg = `@el4cteo/shrink-anything-${process.platform}-${process.arch}`;
const exe = process.platform === "win32" ? "shrink-anything.exe" : "shrink-anything";
let bin;
try {
  bin = require.resolve(`${pkg}/bin/${exe}`);
} catch {
  console.error(
    `shrink-anything: no prebuilt binary for ${process.platform}-${process.arch} (${pkg} isn't installed).\n` +
      "Reinstall without --omit=optional, or build it: cargo install shrink-anything",
  );
  process.exit(1);
}
// Published tarballs have shipped this file as 0644. Set the bit before spawn.
if (process.platform !== "win32") {
  try {
    fs.chmodSync(bin, 0o755);
  } catch {
    // spawnSync reports EACCES if the file still isn't executable
  }
}
const run = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (run.error) {
  console.error(`shrink-anything: ${run.error.message}`);
  process.exit(1);
}
process.exit(run.status ?? 1);
