#!/usr/bin/env node
// Compatibility entry point for older local commands. Linker detection lives
// only in cargo-with-linker.mjs so native and ordinary Cargo commands cannot
// drift apart.
import "./cargo-with-linker.mjs";
