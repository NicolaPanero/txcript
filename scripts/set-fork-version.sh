#!/bin/sh
# Sets the fork's version to <txcript version>-fork.<N> everywhere the apps
# read it: the CLI's `--version` and the transfer helper's engineVersion.
#
#   scripts/set-fork-version.sh 5      # 0.14.4-fork.5
#
# Then commit, and push a matching `v0.14.4-fork.5` tag: fork-release.yml
# publishes the release and starts the Zed and Superset builds.
set -eu
n="$1"
base=$(sed -n 's/^version = "\([0-9.]*\)".*/\1/p' Cargo.toml | head -n 1)
version="$base-fork.$n"
sed -i.bak "3s/^version = \".*\"/version = \"$version\"/" cli/Cargo.toml
sed -i.bak "s/^const ENGINE: &str = \".*\";/const ENGINE: \&str = \"$version\";/" examples/superset-transfer.rs
rm -f cli/Cargo.toml.bak examples/superset-transfer.rs.bak
cargo update --offline -p txcript-cli >/dev/null 2>&1 || cargo metadata --format-version 1 >/dev/null
echo "$version"
