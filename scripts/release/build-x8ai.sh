#!/usr/bin/env bash
# Builds `x8ai` for Apple silicon and Intel Macs as one universal binary, and
# packs it for a release: target/x8ai-release/x8ai-<version>-macos.tar.gz and
# its .sha256. Prints the archive's path. Run on macOS (`lipo`, `codesign`).
set -euo pipefail

cd "$(dirname "$0")/../.."
version="$(cargo pkgid -p x8ai | sed 's/.*[#@]//')"
out=target/x8ai-release
targets=(aarch64-apple-darwin x86_64-apple-darwin)

for target in "${targets[@]}"; do
  rustup target add "$target" >&2
  cargo build --release --locked -p x8ai --target "$target" >&2
done

rm -rf "$out"
mkdir -p "$out"
binaries=()
for target in "${targets[@]}"; do
  binaries+=("target/$target/release/x8ai")
done
lipo -create -output "$out/x8ai" "${binaries[@]}"
# Apple silicon runs only signed code. An ad-hoc signature is enough for a
# program installed from the command line: no quarantine, so no Gatekeeper.
codesign --force --sign - "$out/x8ai"

reported="$("$out/x8ai" --version)"
if [ "$reported" != "x8ai $version" ]; then
  echo "built x8ai reports \"$reported\", expected \"x8ai $version\"" >&2
  exit 1
fi

archive="x8ai-$version-macos.tar.gz"
tar -czf "$out/$archive" -C "$out" x8ai
(cd "$out" && shasum -a 256 "$archive" > "$archive.sha256")
echo "$out/$archive"
