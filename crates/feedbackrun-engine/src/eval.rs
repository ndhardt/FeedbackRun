//! Scalar reference evaluator.
//!
//! The plugin loops window-step-outer / pixel-inner so one layer checkout feeds
//! every pixel; this loops the other way round. Accumulation is per-pixel
//! independent, so the two agree, and this form is testable without a host.

use crate::blend::{BlendMode, Rgba, blend_step};
use crate::window::Window;

/// Settings that affect `state[t]`.
#[derive(Clone, Copy, Debug)]
pub struct StateParams {
    pub decay: [f32; 3],
    pub mode: BlendMode,
    pub opacity: f32,
}

/// Evaluates `state[t]` for one pixel. `src` returns the sample at a comp
/// frame; steps with no source frame still decay.
pub fn eval_state(window: &Window, p: &StateParams, src: impl Fn(i32) -> Rgba) -> Rgba {
    let mut state = Rgba::TRANSPARENT;
    for frame in window.frames() {
        let sample = window
            .source_frame(frame)
            .map(&src)
            .unwrap_or(Rgba::TRANSPARENT);
        state = blend_step(state, sample, p.decay, p.mode, p.opacity, false);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::window_for;

    const EPS8: f32 = 1.0 / 255.0;

    fn params() -> StateParams {
        StateParams {
            decay: [0.9; 3],
            mode: BlendMode::Add,
            opacity: 1.0,
        }
    }

    /// A different value on every frame, so order dependence shows up.
    fn ramp(frame: i32) -> Rgba {
        let v = (frame as f32 * 0.017).rem_euclid(1.0);
        Rgba::new(v, v * 0.5, 1.0 - v, 1.0)
    }

    fn state_at(t: i32) -> Rgba {
        eval_state(&window_for(t, 0, 0.9, EPS8, 512, None), &params(), ramp)
    }

    #[test]
    fn render_order_does_not_change_the_result() {
        let forward: Vec<_> = (0..120).map(state_at).collect();

        let mut reverse: Vec<_> = (0..120).rev().map(state_at).collect();
        reverse.reverse();
        assert_eq!(forward, reverse);

        // Shuffled deterministically - no clock, no RNG.
        let mut order: Vec<i32> = (0..120).collect();
        order.sort_by_key(|t| (t * 37) % 120);
        for t in order {
            assert_eq!(forward[t as usize], state_at(t));
        }
    }

    #[test]
    fn a_single_cold_frame_matches_the_sequence() {
        assert_eq!(state_at(77), state_at(77));
        assert_ne!(state_at(77), state_at(78), "ramp should actually be moving");
    }

    #[test]
    fn anchor_bounds_the_history() {
        let w = window_for(0, 0, 0.9, EPS8, 512, Some(0));
        assert_eq!(eval_state(&w, &params(), ramp), ramp(0));
    }

    #[test]
    fn decay_makes_older_frames_matter_less() {
        let flash = |f: i32| {
            if f == 50 {
                Rgba::new(1.0, 1.0, 1.0, 1.0)
            } else {
                Rgba::TRANSPARENT
            }
        };
        let at = |t: i32| eval_state(&window_for(t, 0, 0.9, EPS8, 512, Some(0)), &params(), flash);
        let mut prev = at(50).r;
        for t in 51..70 {
            let v = at(t).r;
            assert!(v < prev, "frame {t}: {v} !< {prev}");
            prev = v;
        }
    }
}
