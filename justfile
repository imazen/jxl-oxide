# Match the fork's CI toolchain; newer Clippy releases add unrelated style lints.
compatibility-check:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "$HOME/tmp"
    export CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 RUST_TEST_THREADS=4 TMPDIR="$HOME/tmp"
    nice -n 19 cargo +1.95.0 fmt --check
    nice -n 19 cargo +1.95.0 clippy --locked -p jxl-grid -p jxl-modular -p jxl-oxide --all-targets -- -D warnings
    nice -n 19 cargo +1.95.0 test --locked -p jxl-grid -p jxl-modular -p jxl-oxide --lib
    nice -n 19 cargo +1.95.0 test --locked -p jxl-oxide-tests --no-default-features --test test
