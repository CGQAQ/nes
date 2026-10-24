//! Controller reads through the whole machine, with and without the
//! debugger's trace and code/data logger watching.

use nes_core::{Buttons, Config, Nes};

/// NROM program that strobes the pad, shifts its eight buttons into `$00`
/// (first button read ends up in bit 7) and copies the result to `$01`,
/// forever.
fn pad_reader_rom() -> Vec<u8> {
    #[rustfmt::skip]
    let program = [
        0x78,             // $8000 SEI
        0xD8,             //       CLD
        0xA2, 0xFF,       //       LDX #$FF
        0x9A,             //       TXS
        0xA9, 0x01,       // $8005 LDA #1
        0x8D, 0x16, 0x40, //       STA $4016
        0xA9, 0x00,       //       LDA #0
        0x8D, 0x16, 0x40, //       STA $4016
        0xA2, 0x08,       //       LDX #8
        0xAD, 0x16, 0x40, // $8011 LDA $4016
        0x4A,             //       LSR A
        0x26, 0x00,       //       ROL $00
        0xCA,             //       DEX
        0xD0, 0xF7,       //       BNE $8011
        0xA5, 0x00,       //       LDA $00
        0x85, 0x01,       //       STA $01
        0x4C, 0x05, 0x80, //       JMP $8005
    ];
    let mut prg = vec![0xEA; 0x4000];
    prg[..program.len()].copy_from_slice(&program);
    // NMI and IRQ never fire; all vectors point at the reset code.
    prg[0x3FFA..].copy_from_slice(&[0x00, 0x80, 0x00, 0x80, 0x00, 0x80]);

    let mut rom = vec![b'N', b'E', b'S', 0x1A, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    rom.extend(prg);
    rom.extend(vec![0; 0x2000]);
    rom
}

fn read_pad(nes: &mut Nes, buttons: u8) -> u8 {
    nes.set_buttons(0, Buttons(buttons));
    nes.run_frame();
    nes.run_frame();
    nes.peek(0x0001)
}

/// `$01` after reading `buttons`: A lands in bit 7, Right in bit 0.
fn expected(buttons: u8) -> u8 {
    buttons.reverse_bits()
}

fn check(nes: &mut Nes) {
    for buttons in [
        0,
        Buttons::A,
        Buttons::B | Buttons::RIGHT,
        Buttons::START | Buttons::UP | Buttons::LEFT,
        0xFF,
        0,
    ] {
        assert_eq!(
            read_pad(nes, buttons),
            expected(buttons),
            "buttons {buttons:08b}"
        );
    }
}

#[test]
fn reads_buttons() {
    let mut nes = Nes::new(&pad_reader_rom(), Config::default()).unwrap();
    check(&mut nes);
}

#[test]
fn reads_buttons_while_debugging() {
    let mut nes = Nes::new(&pad_reader_rom(), Config::default()).unwrap();
    nes.bus.dbg.trace_enabled = true;
    nes.bus.dbg.cdl_enabled = true;
    check(&mut nes);
}
