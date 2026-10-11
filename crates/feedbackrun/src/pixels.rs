//! Pixel access and the accumulation buffer.
//!
//! AE hands us 8, 16 or 32 bit worlds. The recurrence runs in f32 regardless,
//! or a 50-step accumulation quantises itself to death.

use after_effects as ae;
use feedbackrun_engine as fx;

/// AE's 16-bit worlds are 0..=32768, not 0..=65535.
const MAX16: f32 = ae::MAX_CHANNEL16 as f32;
const MAX8: f32 = 255.0;

pub fn read_pixel(layer: &ae::Layer, x: usize, y: usize) -> fx::Rgba {
    if x >= layer.width() || y >= layer.height() {
        return fx::Rgba::TRANSPARENT;
    }
    match layer.bit_depth() {
        32 => {
            let p = layer.as_pixel32(x, y);
            fx::Rgba::new(p.red, p.green, p.blue, p.alpha)
        }
        16 => {
            let p = layer.as_pixel16(x, y);
            fx::Rgba::new(
                p.red as f32 / MAX16,
                p.green as f32 / MAX16,
                p.blue as f32 / MAX16,
                p.alpha as f32 / MAX16,
            )
        }
        _ => {
            let p = layer.as_pixel8(x, y);
            fx::Rgba::new(
                p.red as f32 / MAX8,
                p.green as f32 / MAX8,
                p.blue as f32 / MAX8,
                p.alpha as f32 / MAX8,
            )
        }
    }
}

pub fn write_pixel(layer: &mut ae::Layer, x: usize, y: usize, depth: i16, v: fx::Rgba) {
    match depth {
        32 => {
            let p = layer.as_pixel32_mut(x, y);
            p.red = v.r;
            p.green = v.g;
            p.blue = v.b;
            p.alpha = v.a;
        }
        16 => {
            let p = layer.as_pixel16_mut(x, y);
            let q = |c: f32| (c * MAX16).round().clamp(0.0, MAX16) as u16;
            p.red = q(v.r);
            p.green = q(v.g);
            p.blue = q(v.b);
            p.alpha = q(v.a);
        }
        _ => {
            let p = layer.as_pixel8_mut(x, y);
            let q = |c: f32| (c * MAX8).round().clamp(0.0, MAX8) as u8;
            p.red = q(v.r);
            p.green = q(v.g);
            p.blue = q(v.b);
            p.alpha = q(v.a);
        }
    }
}

/// Flattens the current source for the display-only post stage.
pub fn snapshot(layer: Option<&ae::Layer>, width: usize, height: usize) -> Vec<fx::Rgba> {
    let mut out = vec![fx::Rgba::TRANSPARENT; width * height];
    if let Some(l) = layer {
        for y in 0..height {
            for x in 0..width {
                out[y * width + x] = read_pixel(l, x, y);
            }
        }
    }
    out
}
