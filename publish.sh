#!/usr/bin/bash
set -xe
./format.sh
cargo +stable build
cargo +stable test
[[ -z "$(git status --porcelain)" ]] || exit 1
cargo publish "$@"
