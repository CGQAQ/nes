use nes_core::Region;
use nes_core::apu::Apu;
#[test]
#[ignore]
fn apu_speed() {
    let mut apu = Apu::new(Region::Ntsc, 48_000.0);
    apu.write(0x4015, 0x0F);
    apu.write(0x4000, 0xBF);
    apu.write(0x4002, 0x40);
    apu.write(0x4003, 0x08);
    let t = std::time::Instant::now();
    let mut out = Vec::new();
    for _ in 0..60 {
        for _ in 0..29780 {
            apu.tick(0.0);
        }
        apu.drain_samples(&mut out);
        out.clear();
    }
    let e = t.elapsed();
    println!(
        "60 frames of APU: {e:?} ({:.1} ns/tick)",
        e.as_nanos() as f64 / (60.0 * 29780.0)
    );
}

#[test]
#[ignore]
fn ppu_and_frame_speed() {
    use nes_core::{Config, Nes, StopReason};
    let rom = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test-data/nes-test-roms/spritecans-2011/spritecans.nes"
    ))
    .unwrap();
    let mut nes = Nes::new(&rom, Config::default()).unwrap();
    for _ in 0..120 {
        while nes.run_frame() != StopReason::FrameComplete {}
    }
    // PPU alone, with whatever state the demo left (rendering on).
    let mut ppu = nes.bus.ppu.clone();
    let mut cart = nes.bus.cart.clone();
    let t = std::time::Instant::now();
    for _ in 0..60 * 89342 {
        ppu.tick(&mut cart);
    }
    let e = t.elapsed();
    println!(
        "PPU: {:.1} ns/dot ({:.2} ms/frame)",
        e.as_nanos() as f64 / (60.0 * 89342.0),
        e.as_secs_f64() * 1000.0 / 60.0
    );
    let t = std::time::Instant::now();
    let mut out = Vec::new();
    for _ in 0..60 {
        while nes.run_frame() != StopReason::FrameComplete {}
        nes.drain_audio(&mut out);
        out.clear();
    }
    println!(
        "whole frame: {:.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 60.0
    );
}
