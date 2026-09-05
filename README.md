# sys-monitor — deprecated snapshot

> **Deprecated:** this repository is retained as historical/source reference. New system-monitor development is maintained in [`quantdale/monitorers`](https://github.com/quantdale/monitorers). Do not start new feature work here unless a task explicitly targets this legacy implementation.

This repository contains an older native Windows system-monitor implementation in Rust, including CPU, memory, disk, network, and GPU monitoring code plus legacy/experimental monitor trees.

## Legacy stack

- Rust
- egui / eframe for the native UI
- `sysinfo` plus Windows-specific metric/platform code

## Historical run/build commands

From the repository root:

```bash
cargo run
cargo build --release
```

The release binary is produced under `target/release/` when the legacy project still builds in the current toolchain/environment.

These commands are documented for maintenance and archaeology only. Their presence is not a fresh build certification.

## Agent and maintenance notes

Read `AGENTS.md` and `.agent/STATE.md` before changing anything. Preserve this repository as a truthful legacy snapshot: fix documentation, security issues, or specifically requested legacy defects when necessary, but do not silently back-port active `monitorers` development or present this repository as the current product.
