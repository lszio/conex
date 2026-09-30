#!/usr/bin/env bash
# scripts/gen-token.sh — sha256 a plaintext token into the form expected by
# host.toml's [[tokens]] block. Used by operators who hold the plaintext
# out-of-band (password manager, vault) and need to paste the hash into
# Dokploy secrets / host.toml.
#
#   scripts/gen-token.sh "<plaintext>"
#
# Prints only the hex digest — never the plaintext — so it is safe to run
# with the output pasted into a CI secret store.

set -euo pipefail

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo "usage: $0 <plaintext-token>" >&2
  exit 64
fi

# sha256sum is in coreutils; openssl is in openssl. Both are in the
# Dockerfile so this works the same locally and in CI.
printf '%s' "$1" | sha256sum | awk '{print $1}'

# Best-effort: scrub the parameter from the shell's argument vector so
# `history` doesn't see it after the process exits.
shift || true
unset -v token || true