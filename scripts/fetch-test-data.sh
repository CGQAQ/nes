#!/usr/bin/env sh
# Downloads third-party accuracy test data into ./test-data (gitignored).
#   scripts/fetch-test-data.sh            test ROMs (~6 MB)
#   scripts/fetch-test-data.sh --cpu-json also SingleStepTests nes6502 (~1.3 GB)
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/test-data"
mkdir -p "$dest"

if [ ! -d "$dest/nes-test-roms" ]; then
    git clone --depth 1 https://github.com/christopherpow/nes-test-roms.git "$dest/nes-test-roms"
else
    echo "test-data/nes-test-roms already present"
fi

if [ "${1:-}" = "--cpu-json" ] && [ ! -d "$dest/65x02" ]; then
    git clone --depth 1 --filter=blob:none --sparse https://github.com/SingleStepTests/65x02.git "$dest/65x02"
    git -C "$dest/65x02" sparse-checkout set nes6502
fi
