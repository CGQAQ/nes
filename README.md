# nes

A cycle-accurate NES/Famicom emulator and debugger written in Rust. Runs on
Windows, macOS and Linux, and in the browser.

- **Accurate**: lockstep, cycle-exact CPU/PPU/APU; passes 135 of 135 suite
  test ROMs plus nestest ([report](docs/compat.md)).
- **Compatible**: 67 iNES mapper numbers, including MMC1/2/3/4/5/6, the VRC
  family, FME-7, Namco 163, Bandai with EEPROM saves, and expansion audio
  (VRC6, VRC7 FM, Sunsoft 5B, Namco 163, MMC5).
- **Built for debugging**: live instruction trace, registers and flags,
  breakpoints, stepping, a code/data-logger-aware disassembler, hex editor,
  and PPU/APU/cartridge viewers.
- Save states, rewind, fast-forward, battery saves, Game Genie, gamepads,
  NTSC/PAL/Dendy timing.

## Running

Requires stable Rust (1.88+).

```sh
cargo run --release -p nes-app -- path/to/game.nes
```

Or launch `nes` without arguments and drop a `.nes` (or `.zip`) file on the
window. The game plays on the screen of a drawn NES-style console: its POWER
button opens a ROM (and power cycles once one is running), RESET resets, the
controller ports light up while their buttons are held, and clicking a port
opens its key bindings. Tools → Settings → Video hides the console. On Linux you need the usual GUI/audio/gamepad development packages
(`libasound2-dev libudev-dev libxkbcommon-dev`).

### In the browser

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cd crates/nes-app && trunk serve      # http://127.0.0.1:8080
```

Load a ROM with the file picker or drag-and-drop. Two URL parameters help
with sharing and testing: `?rom=<url>` fetches and runs a ROM, `?debug`
opens the debugger. Saves and settings live in the browser's local storage.

## Controls

| Key | Action |
|---|---|
| W / A / S / D | D-pad |
| K / J | A / B |
| I / U | Turbo A / turbo B |
| O / L | Start / Select |
| Esc | Pause / resume |
| . | Frame advance |
| Tab (hold) | Fast-forward |
| Backspace (hold) | Rewind |
| 1–0 / Shift+1–0 | Load / save state slot |
| Ctrl+R / Ctrl+Shift+R | Reset / power cycle |
| Ctrl+O | Open ROM |
| Ctrl+D | Toggle debugger panels |
| F5 / F10 / F11 / Shift+F11 | Continue / step over / step into / step out |
| F12 | Screenshot (desktop) |

Keyboard bindings for both players can be changed in Tools → Settings.
Gamepads work out of the box (positional mapping: East = A, South = B).

## Debugger

Debug → *Debugger panels* (Ctrl+D) docks the main tools:

- **CPU** (right): A/X/Y/SP/PC and the flags — editable while paused —
  plus the cycle count, frame, scanline/dot, NMI/IRQ lines and the stack.
  Continue, step into/over/out, and run-to-scanline.
- **Disassembly** (right): follows PC, labels hardware registers and
  interrupt vectors, and uses the code/data logger to show data as `.db`
  instead of decoding garbage. Click the gutter to toggle a breakpoint.
- **Instruction trace** (bottom): every executed instruction with
  registers, effective addresses and the values they hold, scanline/dot and
  cycle — live as the game runs. Can log to a file in nestest.log format.
- **Memory** (bottom): hex editor over the CPU and PPU address spaces,
  internal RAM, OAM, palette RAM, PRG ROM/RAM and CHR. Recent changes are
  highlighted.
- **Breakpoints** (bottom): execute/read/write, single addresses or ranges.
  Debug → *Break on* also stops on NMI, IRQ, BRK or a CPU jam.

The floating viewers show nametables (with the scroll window), pattern
tables with any palette, OAM and the sprite layer, palettes, PPU registers,
per-channel APU oscilloscopes with mute switches, and the cartridge's
header, mapper registers and live bank layout.

## Headless tools

```sh
nes-cli info game.nes
nes-cli run game.nes --frames 600 --screenshot out.png --trace trace.log --press 120:start
nes-cli test test-data/nes-test-roms/ppu_vbl_nmi/rom_singles/*.nes
nes-cli bench game.nes --frames 3000
nes-cli compat-report            # regenerates docs/compat.md
```

`run` prints hashes of the final frame and of all audio produced, which makes
it easy to check that a change is output-neutral.

## Testing

The accuracy test ROMs are third-party and not committed; fetch them once:

```sh
scripts/fetch-test-data.sh        # scripts\fetch-test-data.ps1 on Windows
cargo test --workspace
```

Besides the ROM suite and nestest, `cargo test` builds a synthetic ROM for
every supported mapper, hammers its registers with random writes, and checks
that save states round-trip deterministically.

## Workspace

| Crate | |
|---|---|
| [`nes-core`](crates/nes-core) | The emulator. No I/O or platform dependencies; usable as a library. |
| [`nes-app`](crates/nes-app) | The egui/eframe GUI (binary `nes`), desktop and web. |
| [`nes-cli`](crates/nes-cli) | Headless runner, test harness and benchmarks. |

How the core works:

- **Timing.** Every CPU bus access — dummy reads and writes included —
  first advances the PPU to the exact master-clock position of that access
  (a read lands 5/12 of the way into an NTSC CPU cycle), and ticks the APU
  and mapper. NMI/IRQ are sampled at the end of each cycle with the real
  second-to-last-cycle rule.
- **PPU.** Renders dot by dot with the hardware's fetch sequence, so
  mappers that watch the PPU bus (MMC2's latches, MMC3's A12 counter,
  MMC5's scanline detection) see what they would on a console. Sprite
  evaluation is the per-dot state machine, overflow bug included.
- **DMA.** OAM and DMC DMA steal CPU cycles with the halt, dummy and
  alignment cycles, including the repeated reads that delete controller
  bits or double-read `$2007`.
- **Mappers.** A `Board` holds memory and bank windows; each mapper only
  decides what the windows show. Mappers are an enum dispatched statically,
  which also makes the whole machine serializable for save states.

## License

BSD 3-Clause, see [LICENSE](LICENSE).
