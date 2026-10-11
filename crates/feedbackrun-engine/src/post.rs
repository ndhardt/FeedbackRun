//! Display-only stage. Deliberately outside the accumulation loop so that
//! changing gain or tint never invalidates a cache.

use crate::blend::Rgba;

/// Which buffer to send to the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ShowMode {
    Result = 1,
    StateOnly = 2,
    SourceOnly = 3,
    /// `|state - src|`.
    Difference = 4,
}

impl ShowMode {
    pub const ITEMS: &'static str = "Result|State Only|Source Only|Difference";

    pub fn from_popup(index: i32) -> Self {
        match index {
            2 => Self::StateOnly,
            3 => Self::SourceOnly,
            4 => Self::Difference,
            _ => Self::Result,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PostParams {
    /// 1.0 == 100%.
    pub gain: f32,
    pub tint: [f32; 3],
    /// Mix with source, 0.0 == pure result.
    pub mix: f32,
    pub clamp: bool,
    pub show: ShowMode,
}

impl Default for PostParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            tint: [1.0; 3],
            mix: 0.0,
            clamp: false,
            show: ShowMode::Result,
        }
    }
}

pub fn apply_post(state: Rgba, src: Rgba, p: &PostParams) -> Rgba {
    let mut out = match p.show {
        ShowMode::Result => state,
        ShowMode::StateOnly => state,
        ShowMode::SourceOnly => src,
        ShowMode::Difference => Rgba {
            r: (state.r - src.r).abs(),
            g: (state.g - src.g).abs(),
            b: (state.b - src.b).abs(),
            a: state.a.max(src.a),
        },
    };

    if p.show != ShowMode::SourceOnly {
        out.r *= p.gain * p.tint[0];
        out.g *= p.gain * p.tint[1];
        out.b *= p.gain * p.tint[2];
    }

    if p.mix > 0.0 && p.show == ShowMode::Result {
        out = out.lerp(src, p.mix);
    }

    if p.clamp {
        out.r = out.r.clamp(0.0, 1.0);
        out.g = out.g.clamp(0.0, 1.0);
        out.b = out.b.clamp(0.0, 1.0);
        out.a = out.a.clamp(0.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_a_pass_through() {
        let s = Rgba::new(0.3, 0.6, 0.9, 1.0);
        assert_eq!(apply_post(s, Rgba::TRANSPARENT, &PostParams::default()), s);
    }

    #[test]
    fn full_mix_returns_the_source() {
        let s = Rgba::new(0.3, 0.6, 0.9, 1.0);
        let src = Rgba::new(0.1, 0.1, 0.1, 1.0);
        let p = PostParams {
            mix: 1.0,
            ..Default::default()
        };
        assert_eq!(apply_post(s, src, &p), src);
    }

    #[test]
    fn difference_of_identical_buffers_is_black() {
        let s = Rgba::new(0.3, 0.6, 0.9, 1.0);
        let p = PostParams {
            show: ShowMode::Difference,
            ..Default::default()
        };
        let out = apply_post(s, s, &p);
        assert!(out.r == 0.0 && out.g == 0.0 && out.b == 0.0);
    }

    #[test]
    fn hdr_survives_unless_clamped() {
        let s = Rgba::new(2.0, 2.0, 2.0, 1.0);
        assert!(apply_post(s, s, &PostParams::default()).r > 1.0);
        let p = PostParams {
            clamp: true,
            ..Default::default()
        };
        assert_eq!(apply_post(s, s, &p).r, 1.0);
    }
}
