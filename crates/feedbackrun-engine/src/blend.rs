//! Blend modes and the accumulation step. Linear f32, straight alpha.
//! Values above 1.0 are not clamped here; see [`crate::post`].

/// Linear f32 RGBA sample.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Alpha decays by the mean of the RGB factors so unlinked channels still
    /// fade out.
    #[inline]
    pub fn decay(self, dr: f32, dg: f32, db: f32) -> Self {
        let da = (dr + dg + db) / 3.0;
        Self {
            r: self.r * dr,
            g: self.g * dg,
            b: self.b * db,
            a: self.a * da,
        }
    }

    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        // Exact at the endpoints - a 100% mix must return the source bit for bit.
        if t <= 0.0 {
            return self;
        }
        if t >= 1.0 {
            return other;
        }
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    #[inline]
    pub fn scale(self, amount: f32) -> Self {
        Self {
            r: self.r * amount,
            g: self.g * amount,
            b: self.b * amount,
            a: self.a * amount,
        }
    }

    #[inline]
    fn zip(self, other: Self, f: impl Fn(f32, f32) -> f32) -> Self {
        Self {
            r: f(self.r, other.r),
            g: f(self.g, other.g),
            b: f(self.b, other.b),
            a: f(self.a, other.a),
        }
    }
}

/// Discriminants are AE's 1-based popup indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum BlendMode {
    Over = 1,
    Add = 2,
    Screen = 3,
    Lighten = 4,
    Darken = 5,
    Average = 6,
    Difference = 7,
    Multiply = 8,
    Replace = 9,
    FeedbackOnly = 10,
    Under = 11,
    Blend = 12,
    Overlay = 13,
    SoftLight = 14,
    HardLight = 15,
    Color = 16,
    Luminosity = 17,
}

impl BlendMode {
    pub const ITEMS: &'static str = "Over (Composite Over)|Add|Screen|Maximum / Lighten|Minimum / Darken|Average|Difference|Multiply|Source Only|Feedback Only|Under (Composite Under)|Blend|Overlay|Soft Light|Hard Light|Color|Luminosity";

    pub fn from_popup(index: i32) -> Self {
        match index {
            2 => Self::Add,
            3 => Self::Screen,
            4 => Self::Lighten,
            5 => Self::Darken,
            6 => Self::Average,
            7 => Self::Difference,
            8 => Self::Multiply,
            9 => Self::Replace,
            10 => Self::FeedbackOnly,
            11 => Self::Under,
            12 => Self::Blend,
            13 => Self::Overlay,
            14 => Self::SoftLight,
            15 => Self::HardLight,
            16 => Self::Color,
            17 => Self::Luminosity,
            _ => Self::Over,
        }
    }

    pub fn is_order_sensitive(self) -> bool {
        matches!(
            self,
            Self::Over
                | Self::Under
                | Self::Overlay
                | Self::SoftLight
                | Self::HardLight
                | Self::Color
                | Self::Luminosity
        )
    }

    /// `state` is the backdrop, `src` the foreground.
    #[inline]
    pub fn apply(self, state: Rgba, src: Rgba) -> Rgba {
        match self {
            Self::Over => alpha_over(src, state),
            Self::Add => state.zip(src, |d, s| d + s),
            Self::Screen => state.zip(src, |d, s| 1.0 - (1.0 - d) * (1.0 - s)),
            Self::Lighten => state.zip(src, f32::max),
            Self::Darken => state.zip(src, f32::min),
            Self::Average => state.zip(src, |d, s| (d + s) * 0.5),
            Self::Difference => Rgba {
                r: (state.r - src.r).abs(),
                g: (state.g - src.g).abs(),
                b: (state.b - src.b).abs(),
                a: 1.0,
            },
            Self::Multiply => state.zip(src, |d, s| d * s),
            Self::Replace => src,
            Self::FeedbackOnly => state,
            Self::Under => alpha_over(state, src),
            Self::Blend => state.lerp(src, 0.5),
            Self::Overlay => artistic_blend(state, src, |d, s| overlay(d, s)),
            Self::SoftLight => artistic_blend(state, src, soft_light),
            Self::HardLight => artistic_blend(state, src, |d, s| overlay(s, d)),
            Self::Color => color_blend(state, src, false),
            Self::Luminosity => color_blend(state, src, true),
        }
    }
}

fn alpha_over(front: Rgba, back: Rgba) -> Rgba {
    let fa = front.a.clamp(0.0, 1.0);
    let ba = back.a.clamp(0.0, 1.0);
    let a = fa + ba * (1.0 - fa);
    if a <= 0.0 {
        return Rgba::TRANSPARENT;
    }
    let mix = |f: f32, b: f32| (f * fa + b * ba * (1.0 - fa)) / a;
    Rgba {
        r: mix(front.r, back.r),
        g: mix(front.g, back.g),
        b: mix(front.b, back.b),
        a,
    }
}

#[inline]
fn overlay(base: f32, blend: f32) -> f32 {
    if base <= 0.5 {
        2.0 * base * blend
    } else {
        1.0 - 2.0 * (1.0 - base) * (1.0 - blend)
    }
}

#[inline]
fn soft_light(base: f32, blend: f32) -> f32 {
    if blend <= 0.5 {
        base - (1.0 - 2.0 * blend) * base * (1.0 - base)
    } else {
        let curve = if base <= 0.25 {
            ((16.0 * base - 12.0) * base + 4.0) * base
        } else {
            base.max(0.0).sqrt()
        };
        base + (2.0 * blend - 1.0) * (curve - base)
    }
}

fn artistic_blend(state: Rgba, src: Rgba, op: impl Fn(f32, f32) -> f32) -> Rgba {
    let color = Rgba {
        r: op(state.r, src.r),
        g: op(state.g, src.g),
        b: op(state.b, src.b),
        a: src.a,
    };
    alpha_over(color, state)
}

fn luminance(rgb: [f32; 3]) -> f32 {
    0.3 * rgb[0] + 0.59 * rgb[1] + 0.11 * rgb[2]
}

fn set_luminance(mut rgb: [f32; 3], target: f32) -> [f32; 3] {
    let shift = target - luminance(rgb);
    for c in &mut rgb {
        *c += shift;
    }
    // The usual Color/Luminosity clipping assumes SDR. Keep the shifted
    // linear values for HDR instead of reversing their sign around 1.0.
    if !(0.0..=1.0).contains(&target) {
        return rgb;
    }
    let low = rgb.iter().copied().fold(f32::INFINITY, f32::min);
    if low < 0.0 {
        for c in &mut rgb {
            *c = target + (*c - target) * target / (target - low).max(1e-6);
        }
    }
    let high = rgb.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if high > 1.0 {
        for c in &mut rgb {
            *c = target + (*c - target) * (1.0 - target) / (high - target).max(1e-6);
        }
    }
    rgb
}

fn color_blend(state: Rgba, src: Rgba, luminosity: bool) -> Rgba {
    let base = [state.r, state.g, state.b];
    let blend = [src.r, src.g, src.b];
    let rgb = if luminosity {
        set_luminance(base, luminance(blend))
    } else {
        set_luminance(blend, luminance(base))
    };
    alpha_over(
        Rgba {
            r: rgb[0],
            g: rgb[1],
            b: rgb[2],
            a: src.a,
        },
        state,
    )
}

/// Decay the accumulator, then blend the source in. `opacity` in 0..=1;
/// at 0 nothing is blended and only the decay trail remains.
#[inline]
pub fn blend_step(
    state: Rgba,
    src: Rgba,
    decay: [f32; 3],
    mode: BlendMode,
    opacity: f32,
    swap_order: bool,
) -> Rgba {
    let amount = opacity.clamp(0.0, 1.0);
    if mode == BlendMode::Blend {
        // Blend is a temporal crossfade. Accumulation controls how much of the
        // existing buffer survives; Input Amount limits the new contribution.
        let dr = decay[0].clamp(0.0, 1.0);
        let dg = decay[1].clamp(0.0, 1.0);
        let db = decay[2].clamp(0.0, 1.0);
        let da = (dr + dg + db) / 3.0;
        return Rgba {
            r: state.r * dr + src.r * (1.0 - dr) * amount,
            g: state.g * dg + src.g * (1.0 - dg) * amount,
            b: state.b * db + src.b * (1.0 - db) * amount,
            a: state.a * da + src.a * (1.0 - da) * amount,
        };
    }
    let decayed = state.decay(decay[0], decay[1], decay[2]);
    // The reset frame passes the live input through. Otherwise modes such as
    // Multiply, Darken and Feedback Only can never acquire an initial image.
    if decayed.a <= 0.0 {
        return src.scale(amount);
    }
    if mode == BlendMode::Replace {
        return src.scale(amount);
    }
    if mode == BlendMode::FeedbackOnly {
        return decayed;
    }
    let blended = if swap_order {
        mode.apply(src, decayed)
    } else {
        mode.apply(decayed, src)
    };
    if amount >= 1.0 {
        blended
    } else {
        decayed.lerp(blended, amount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPAQUE_WHITE: Rgba = Rgba::new(1.0, 1.0, 1.0, 1.0);

    #[test]
    fn over_onto_empty_returns_source() {
        let out = BlendMode::Over.apply(Rgba::TRANSPARENT, OPAQUE_WHITE);
        assert_eq!(out, OPAQUE_WHITE);
    }

    #[test]
    fn over_with_transparent_source_keeps_state() {
        let state = Rgba::new(0.5, 0.25, 0.125, 0.5);
        let out = BlendMode::Over.apply(state, Rgba::TRANSPARENT);
        assert!((out.a - state.a).abs() < 1e-6);
        assert!((out.r - state.r).abs() < 1e-6);
    }

    #[test]
    fn zero_opacity_is_pure_decay() {
        let state = OPAQUE_WHITE;
        let out = blend_step(state, OPAQUE_WHITE, [0.5; 3], BlendMode::Add, 0.0, false);
        assert!((out.r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn replace_ignores_state() {
        let out = blend_step(
            OPAQUE_WHITE,
            Rgba::new(0.2, 0.2, 0.2, 1.0),
            [0.9; 3],
            BlendMode::Replace,
            1.0,
            false,
        );
        assert!((out.r - 0.2).abs() < 1e-6);
    }

    #[test]
    fn hdr_values_survive_add() {
        let out = BlendMode::Add.apply(OPAQUE_WHITE, OPAQUE_WHITE);
        assert!(out.r > 1.0, "Add must not clamp");
    }

    #[test]
    fn per_channel_decay_produces_fringing() {
        let out = OPAQUE_WHITE.decay(0.9, 0.5, 0.1);
        assert!(out.r > out.g && out.g > out.b);
    }
}
