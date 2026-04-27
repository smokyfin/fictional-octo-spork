#!/usr/bin/env bash
# Build the Rust core as a fat staticlib for iOS (device + simulator) and
# stage it next to the PacketTunnel extension.
#
# Requires:
#   - rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim

set -euo pipefail

cd "$(dirname "$0")/../rust"

cargo build --release --target aarch64-apple-ios -p ff_vpn_core
cargo build --release --target x86_64-apple-ios -p ff_vpn_core
cargo build --release --target aarch64-apple-ios-sim -p ff_vpn_core

OUT="../flutter_app/ios/PacketTunnel/Frameworks"
mkdir -p "$OUT"

# Device.
cp target/aarch64-apple-ios/release/libff_vpn_core.a "$OUT/libff_vpn_core_device.a"

# Simulator universal (arm64 + x86_64).
lipo -create \
    target/aarch64-apple-ios-sim/release/libff_vpn_core.a \
    target/x86_64-apple-ios/release/libff_vpn_core.a \
    -output "$OUT/libff_vpn_core_sim.a"

echo "Built static libs in $OUT"
