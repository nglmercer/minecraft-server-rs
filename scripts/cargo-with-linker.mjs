#!/usr/bin/env node
/**
 * Run Cargo with an optional Windows LLVM linker.
 *
 * Precedence for the Windows MSVC targets is:
 *   1. an explicitly configured CARGO_TARGET_*_LINKER value
 *   2. lld-link when it is available on PATH
 *   3. Cargo/Rust's default MSVC linker
 *
 * Linux and macOS inherit the normal environment unchanged.
 *
 * With no arguments this keeps the project's native release build as the
 * default. Otherwise every argument is forwarded to Cargo, so the same
 * wrapper can run check, clippy, test, or any other Cargo command.
 */
import { spawnSync } from "node:child_process";

const LINKER_VARS = [
  "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER",
  "CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_LINKER",
];

function commandExists(command) {
  const checker = process.platform === "win32" ? "where" : "which";
  return (
    spawnSync(checker, [command], {
      stdio: "ignore",
    }).status === 0
  );
}

function configureWindowsLinker(env) {
  if (process.platform !== "win32") {
    return;
  }

  const configured = LINKER_VARS.filter((name) => env[name] !== undefined);
  if (!commandExists("lld-link")) {
    if (configured.length > 0) {
      console.log(`[cargo] Using configured linker: ${configured.join(", ")}`);
    } else {
      console.log(
        "[cargo] lld-link not found; using the default Rust/MSVC linker",
      );
    }
    return;
  }

  const detected = [];
  for (const name of LINKER_VARS) {
    if (env[name] === undefined) {
      env[name] = "lld-link";
      detected.push(name);
    }
  }

  if (configured.length > 0 && detected.length > 0) {
    console.log(
      `[cargo] Using configured linker: ${configured.join(", ")}; ` +
        `using lld-link for: ${detected.join(", ")}`,
    );
  } else if (configured.length > 0) {
    console.log(`[cargo] Using configured linker: ${configured.join(", ")}`);
  } else {
    console.log("[cargo] Using LLVM lld-link");
  }
}

const env = { ...process.env };
configureWindowsLinker(env);

const args = process.argv.slice(2);
const cargoArgs =
  args.length > 0 ? args : ["build", "--release", "--locked", "-p", "panel"];

const result = spawnSync("cargo", cargoArgs, {
  env,
  stdio: "inherit",
  // On Windows Cargo may be exposed as cargo.cmd rather than a directly
  // executable binary. Keep the existing Windows invocation behavior.
  shell: process.platform === "win32",
});

if (result.error) {
  console.error(result.error);
  process.exit(1);
}

process.exit(result.status ?? 1);
