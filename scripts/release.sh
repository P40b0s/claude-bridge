#!/usr/bin/env bash
# Bumps cli/Cargo.toml to the given version, commits it and pushes the commit and the
# tag together, so the tag always points at the commit that carries the version.
#   scripts/release.sh 0.1.6
set -euo pipefail

version="${1:?usage: scripts/release.sh <version, e.g. 0.1.6>}"
version="${version#v}"
tag="v$version"
cd "$(git rev-parse --show-toplevel)"

if [ -n "$(git status --porcelain)" ]; then
  echo "commit or stash your changes first" >&2
  exit 1
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists; to move it: git tag -d $tag && git push origin :refs/tags/$tag" >&2
  exit 1
fi

current=$(sed -n 's/^version = "\(.*\)"/\1/p' cli/Cargo.toml | head -1)
if [ "$current" != "$version" ]; then
  sed -i.bak "0,/^version = \".*\"/s//version = \"$version\"/" cli/Cargo.toml
  rm cli/Cargo.toml.bak
  cargo update -p claude-bridge --offline >/dev/null 2>&1 || cargo metadata --format-version 1 >/dev/null
  git commit -qm "release $tag" cli/Cargo.toml Cargo.lock
fi

git tag "$tag"
git push --atomic origin HEAD "$tag"
echo "pushed $tag at $(git rev-parse --short HEAD)"
