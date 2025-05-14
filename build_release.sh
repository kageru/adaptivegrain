#!/bin/sh

# Haswell was the first generation to have FMA instructions, which this plugin makes heavy use of. There’s about a 20% speedup from this.
# I doubt anyone still encodes on hardware older than that, and if they do, I trust they can figure out how to build their own binaries.
# --emit=asm forces rustc to compile the crate with only one thread, which can help the optimizer (1-2% faster on my machine)
RUSTFLAGS="-C target-cpu=haswell --emit asm" cargo build --release --locked
RUSTFLAGS="-C target-cpu=haswell --emit asm" cargo build --release --target=x86_64-pc-windows-gnu --locked
mv target/x86_64-pc-windows-gnu/release/adaptivegrain_rs.dll ./
mv target/release/libadaptivegrain_rs.so ./
strip libadaptivegrain_rs.so
strip adaptivegrain_rs.dll
