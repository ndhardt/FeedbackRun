//! Bounded evaluation window.
//!
//! `state[t]` depends on a finite range of source frames and nothing else, so
//! any frame can be rendered on its own, out of order, with a cold cache.
//!
//! With `decay < 1` the tail is cut where it drops below the output
//! quantisation step. `decay == 1` has no such bound and is clamped to
//! `max_window`.

/// How to sample source frames before the anchor during warm-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum PreAnchorMode {
    /// Nothing exists before the anchor; those steps are skipped.
    Empty = 1,
    /// The anchor frame is held.
    Hold = 2,
    /// Time reflects around the anchor.
    Mirror = 3,
}

impl PreAnchorMode {
    pub const ITEMS: &'static str = "Empty|Hold|Mirror";

    pub fn from_popup(index: i32) -> Self {
        match index {
            2 => Self::Hold,
            3 => Self::Mirror,
            _ => Self::Empty,
        }
    }
}

/// Quantisation step for an output bit depth. The tail is cut where `decay^N`
/// can no longer move the least significant bit.
pub fn truncation_epsilon(bits: u32) -> f32 {
    match bits {
        0 => 1.0 / 255.0, // Auto / unknown: assume 8 bit.
        b => 1.0 / ((1u64 << b.min(24)) - 1) as f32,
    }
}

/// Number of frames the accumulator remembers before falling under ε.
pub fn truncation_length(decay: f32, epsilon: f32, max_window: i32) -> i32 {
    if decay.is_nan() || decay <= 0.0 {
        return 1; // Fully decayed: only the current frame contributes.
    }
    if decay >= 1.0 {
        return max_window.max(1);
    }
    let n = (epsilon.ln() / decay.ln()).ceil();
    if !n.is_finite() {
        return max_window.max(1);
    }
    (n as i32).clamp(1, max_window.max(1))
}

/// The closed range of frames feeding `state[t]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// First frame evaluated (may be before `anchor` when warming up).
    pub start: i32,
    /// Last frame evaluated, always the frame being rendered.
    pub end: i32,
    pub anchor: i32,
    pub pre_anchor: PreAnchorMode,
    /// Set when the window hit `max_window` rather than the epsilon tail.
    pub truncated_by_budget: bool,
}

impl Window {
    /// Frames evaluated, oldest first.
    pub fn len(&self) -> i32 {
        self.end - self.start + 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 0
    }

    /// Source frame to check out for a window step. `None` contributes
    /// nothing; the accumulator still decays.
    pub fn source_frame(&self, frame: i32) -> Option<i32> {
        if frame >= self.anchor {
            return Some(frame);
        }
        match self.pre_anchor {
            PreAnchorMode::Empty => None,
            PreAnchorMode::Hold => Some(self.anchor),
            PreAnchorMode::Mirror => Some(self.anchor + (self.anchor - frame)),
        }
    }

    pub fn frames(&self) -> impl Iterator<Item = i32> + use<> {
        self.start..=self.end
    }
}

/// Builds the window for frame `t`. `warmup` frames are evaluated ahead of the
/// anchor to hide the ramp-up; `None` means the full tail length.
pub fn window_for(
    t: i32,
    anchor: i32,
    decay: f32,
    epsilon: f32,
    max_window: i32,
    warmup: Option<i32>,
) -> Window {
    let tail = truncation_length(decay, epsilon, max_window);
    let warmup = warmup.unwrap_or(tail).max(0);

    let earliest = anchor - warmup;
    let start = (t - tail + 1).max(earliest);
    let truncated_by_budget = decay >= 1.0 && start > earliest;

    Window {
        start: start.min(t),
        end: t,
        anchor,
        pre_anchor: PreAnchorMode::Empty,
        truncated_by_budget,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS8: f32 = 1.0 / 255.0;

    #[test]
    fn classic_decay_bounds_the_window() {
        assert_eq!(truncation_length(0.9, EPS8, 4096), 53);
        assert!(truncation_length(0.99, EPS8, 4096) > truncation_length(0.9, EPS8, 4096));
    }

    #[test]
    fn zero_decay_remembers_only_now() {
        let w = window_for(100, 0, 0.0, EPS8, 4096, Some(0));
        assert_eq!(w.len(), 1);
        assert_eq!(w.start, 100);
    }

    #[test]
    fn window_never_starts_before_anchor_minus_warmup() {
        let w = window_for(10, 5, 0.99, EPS8, 4096, Some(2));
        assert_eq!(w.start, 3);
    }

    #[test]
    fn unity_decay_is_budget_limited_and_flagged() {
        let w = window_for(10_000, 0, 1.0, EPS8, 512, Some(0));
        assert_eq!(w.len(), 512);
        assert!(w.truncated_by_budget);
    }

    #[test]
    fn window_is_a_pure_function_of_its_inputs() {
        let a = window_for(42, 0, 0.9, EPS8, 4096, None);
        let b = window_for(42, 0, 0.9, EPS8, 4096, None);
        assert_eq!(a, b);
    }

    #[test]
    fn pre_anchor_modes_map_time_as_documented() {
        let mut w = window_for(10, 5, 0.9, EPS8, 4096, Some(3));
        assert_eq!(w.source_frame(7), Some(7));

        w.pre_anchor = PreAnchorMode::Empty;
        assert_eq!(w.source_frame(3), None);
        w.pre_anchor = PreAnchorMode::Hold;
        assert_eq!(w.source_frame(3), Some(5));
        w.pre_anchor = PreAnchorMode::Mirror;
        assert_eq!(w.source_frame(3), Some(7));
    }
}
