# sys-monitor

A native Windows system monitoring application built in Rust.
Displays real-time CPU and memory usage with scrolling graphs.

## Stack

- Rust
- egui / eframe (immediate mode GUI)
- sysinfo (system metrics)

## How to run

cargo run

## How to build a standalone .exe

cargo build --release

# Output: target/release/sys-monitor.exe
