//! NES colors.
//!
//! The PPU outputs a composite video signal rather than RGB, so the default
//! palette is derived the same way a TV would: synthesize the 2C02's square
//! wave for each color (including the emphasis bits' attenuation), and
//! demodulate it into YUV. This yields all 512 color/emphasis combinations.

/// Palette RAM contents at power-up, as measured on a 2C02G.
pub const POWER_UP_PALETTE: [u8; 32] = [
    0x09, 0x01, 0x00, 0x01, 0x00, 0x02, 0x02, 0x0D, 0x08, 0x10, 0x08, 0x24, 0x00, 0x00, 0x04, 0x2C,
    0x09, 0x01, 0x34, 0x03, 0x00, 0x04, 0x00, 0x14, 0x08, 0x3A, 0x00, 0x02, 0x00, 0x20, 0x2C, 0x08,
];

/// RGBA lookup table indexed by a framebuffer value (`emphasis << 6 | color`).
#[derive(Clone)]
pub struct Palette {
    pub rgba: Box<[[u8; 4]; 512]>,
}

impl Default for Palette {
    fn default() -> Self {
        Self::ntsc()
    }
}

// Signal voltages relative to sync, from measurements of a 2C02.
const BLACK: f32 = 0.518;
const WHITE: f32 = 1.962;
const ATTENUATION: f32 = 0.746;
const LEVELS: [f32; 8] = [0.350, 0.518, 0.962, 1.550, 1.094, 1.506, 1.962, 1.962];

fn signal(pixel: usize, phase: usize, emphasis_swap_rg: bool) -> f32 {
    let color = pixel & 0x0F;
    let mut level = (pixel >> 4) & 3;
    let mut emphasis = pixel >> 6;
    if emphasis_swap_rg {
        emphasis = (emphasis & 4) | ((emphasis & 1) << 1) | ((emphasis & 2) >> 1);
    }
    if color > 13 {
        level = 1;
    }
    let mut low = LEVELS[level];
    let mut high = LEVELS[4 + level];
    if color == 0 {
        low = high;
    }
    if color > 12 {
        high = low;
    }
    let in_phase = |c: usize| (c + phase) % 12 < 6;
    let mut s = if in_phase(color) { high } else { low };
    if (emphasis & 1 != 0 && in_phase(0))
        || (emphasis & 2 != 0 && in_phase(4))
        || (emphasis & 4 != 0 && in_phase(8))
    {
        s *= ATTENUATION;
    }
    s
}

fn generate(emphasis_swap_rg: bool) -> Palette {
    let mut rgba = Box::new([[0u8; 4]; 512]);
    for (pixel, out) in rgba.iter_mut().enumerate() {
        let (mut y, mut u, mut v) = (0.0f32, 0.0f32, 0.0f32);
        for p in 0..12 {
            let s = (signal(pixel, p, emphasis_swap_rg) - BLACK) / (WHITE - BLACK);
            // Phase angle such that the color burst (hue 8) decodes to 180°
            // and hues advance clockwise, as on a TV.
            let angle = (p as f32 - 0.5) * std::f32::consts::PI / 6.0;
            y += s;
            u += s * angle.cos();
            v -= s * angle.sin();
        }
        y /= 12.0;
        u *= 2.0 / 12.0;
        v *= 2.0 / 12.0;
        let r = y + 1.139_83 * v;
        let g = y - 0.394_65 * u - 0.580_60 * v;
        let b = y + 2.032_11 * u;
        let to8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        *out = [to8(r), to8(g), to8(b), 255];
    }
    Palette { rgba }
}

impl Palette {
    /// The default 2C02 (NTSC) palette.
    pub fn ntsc() -> Self {
        generate(false)
    }

    /// 2C07 (PAL) palette: the red and green emphasis bits are swapped.
    pub fn pal() -> Self {
        generate(true)
    }

    /// Loads a `.pal` file: 64 RGB triplets (192 bytes, emphasis derived by
    /// attenuating channels) or 512 triplets (1536 bytes, emphasis included).
    pub fn from_pal_file(data: &[u8]) -> Option<Self> {
        let mut rgba = Box::new([[0u8; 4]; 512]);
        match data.len() {
            1536 => {
                for (i, c) in data.as_chunks::<3>().0.iter().enumerate() {
                    rgba[i] = [c[0], c[1], c[2], 255];
                }
            }
            192 => {
                for (i, out) in rgba.iter_mut().enumerate() {
                    let c = &data[(i & 0x3F) * 3..(i & 0x3F) * 3 + 3];
                    let e = i >> 6;
                    let mut rgb = [c[0] as f32, c[1] as f32, c[2] as f32];
                    // Emphasizing one channel attenuates the other two.
                    for (ch, bit) in [(0usize, 1usize), (1, 2), (2, 4)] {
                        if e & bit != 0 {
                            for (other, val) in rgb.iter_mut().enumerate() {
                                if other != ch {
                                    *val *= ATTENUATION;
                                }
                            }
                        }
                    }
                    *out = [rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, 255];
                }
            }
            _ => return None,
        }
        Some(Palette { rgba })
    }

    #[inline]
    pub fn rgba(&self, pixel: u16) -> [u8; 4] {
        self.rgba[(pixel & 0x1FF) as usize]
    }

    /// Converts a whole framebuffer to packed RGBA bytes.
    pub fn convert(&self, frame: &[u16], out: &mut Vec<u8>) {
        out.clear();
        out.reserve(frame.len() * 4);
        for &p in frame {
            out.extend_from_slice(&self.rgba(p));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hues_land_in_the_right_place() {
        let p = Palette::ntsc();
        // $0F is black, $30 white, $12 blue, $16 red, $1A green.
        assert_eq!(p.rgba(0x0F)[..3], [0, 0, 0]);
        let white = p.rgba(0x30);
        assert!(white[..3].iter().all(|&c| c > 230), "{white:?}");
        let blue = p.rgba(0x12);
        assert!(blue[2] > blue[0] && blue[2] > blue[1], "{blue:?}");
        let red = p.rgba(0x16);
        assert!(red[0] > red[1] && red[0] > red[2], "{red:?}");
        let green = p.rgba(0x1A);
        assert!(green[1] > green[0] && green[1] > green[2], "{green:?}");
    }
}
