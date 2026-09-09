#!/usr/bin/env bash
# Install build dependencies from the runner's Ubuntu distribution sources only.
# Invoke with sudo on Ubuntu runners that provide a deb822 ubuntu.sources file.
set -euo pipefail

ubuntu_sources=/etc/apt/sources.list.d/ubuntu.sources
if [[ ! -r "$ubuntu_sources" ]]; then
  echo "Expected the Ubuntu runner's distribution sources at $ubuntu_sources" >&2
  exit 2
fi

apt_root=$(mktemp -d "${TMPDIR:-/tmp}/kv9-ci-apt.XXXXXXXX")
trap 'rm -rf -- "$apt_root"' EXIT
# APT's unprivileged download worker must be able to traverse these directories.
chmod 0755 "$apt_root"
mkdir -m 0755 "$apt_root/sources" "$apt_root/lists"
install -m 0644 "$ubuntu_sources" "$apt_root/sources/ubuntu.sources"

# Keep the runner's mirror and Signed-By settings, without modifying its sources.
# Fresh lists and disabled package caches also exclude stale third-party metadata.
# Use exactly the same configuration for update and install. Any selected-source
# update error remains fatal; signature and package-hash verification stay enabled.
apt_options=(
  -o Dir::Etc::sourcelist=/dev/null
  -o "Dir::Etc::sourceparts=$apt_root/sources"
  -o "Dir::State::lists=$apt_root/lists"
  -o Dir::Cache::pkgcache=
  -o Dir::Cache::srcpkgcache=
  -o APT::Update::Error-Mode=any
  -o Acquire::Retries=3
)
apt-get "${apt_options[@]}" update
DEBIAN_FRONTEND=noninteractive apt-get "${apt_options[@]}" install \
  --yes --no-install-recommends protobuf-compiler cmake build-essential

protoc --version
cmake --version
cc --version
