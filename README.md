# nes

A cycle-accurate NES emulator and debugger written in Rust, for Windows,
macOS, Linux and the browser.

> Status: work in progress. The emulation core (CPU, PPU, APU, DMA) is
> complete and passes the accuracy suites listed below; the GUI, debugger
> panels and the full mapper set are being built.

## Workspace

| Crate | What it is |
|---|---|
| [`crates/nes-core`](crates/nes-core) | The emulator: CPU, PPU, APU, cartridges, debugger hooks, save states. No I/O or platform dependencies. |
| [`crates/nes-cli`](crates/nes-cli) | Headless runner: screenshots, instruction traces, test ROMs, benchmarks. |
| [`crates/nes-app`](crates/nes-app) | The GUI (egui/eframe), native and web. |

## Accuracy

The core is lockstep cycle-accurate: every CPU bus access — including dummy
reads and writes — advances the PPU and APU to the exact master-clock
position of that access. The PPU renders dot by dot with the real fetch
pattern, and OAM/DMC DMA steal cycles with the hardware's halt, dummy and
alignment cycles.

Passing today (`cargo test -p nes-core --test test_roms`):

- **nestest**: all 8,991 trace lines, including unofficial opcodes and cycle counts
- **CPU**: instr_test-v5 (all 16), instr_misc, instr_timing, cpu_interrupts_v2 (all 5), cpu_dummy_writes, cpu_exec_space, cpu_reset
- **PPU**: ppu_vbl_nmi (all 10), vbl_nmi_timing (all 7), sprite_hit_tests (all 11), sprite_overflow_tests (all 5), ppu_open_bus, oam_read, oam_stress, blargg 2005 PPU tests
- **APU**: apu_test (all 8), apu_reset (all 6), sprdma_and_dmc_dma

## Building

Requires a stable Rust toolchain (1.88+).

```sh
cargo build --release
cargo run --release -p nes-cli -- info game.nes
cargo run --release -p nes-cli -- run game.nes --frames 600 --screenshot out.png
```

## Testing

Accuracy test ROMs are third-party and not committed. Fetch them once:

```sh
scripts/fetch-test-data.sh        # or scripts\fetch-test-data.ps1 on Windows
cargo test --workspace --release
```

Run any `$6000`-protocol test ROM directly:

```sh
cargo run --release -p nes-cli -- test test-data/nes-test-roms/ppu_vbl_nmi/rom_singles/*.nes
```

## License

BSD 3-Clause, see [LICENSE](LICENSE).
