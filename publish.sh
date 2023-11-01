#!/usr/bin/bash
set -xe
mdformat . --wrap 80
cargo fmt
cargo +stable build
cargo +stable test
[[ -z "$(git status --porcelain)" ]] || exit 1
cargo publish "$@"
