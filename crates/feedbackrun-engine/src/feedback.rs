//! Exact frame-to-frame feedback with an optional transform of the previous state.
//! The caller supplies frames in time order, starting at a known anchor or checkpoint.

use crate::{BlendMode, Rgba, blend_step};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedbackTransform {
    /// Movement of the previous state in output pixels per frame.
    pub x: f32,
    pub y: f32,
    /// 1.0 leaves the previous state unchanged.
    pub scale: f32,
    pub rotation_degrees: f32,
    /// Box blur radius, in pixels. Applied to the previous state only.
    pub blur_radius: u8,
}

impl Default for FeedbackTransform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            rotation_degrees: 0.0,
            blur_radius: 0,
        }
    }
}

impl FeedbackTransform {
    pub fn is_identity(self) -> bool {
        self.x == 0.0
            && self.y == 0.0
            && self.scale == 1.0
            && self.rotation_degrees == 0.0
            && self.blur_radius == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedbackStep {
    pub decay: [f32; 3],
    pub mode: BlendMode,
    pub opacity: f32,
    pub swap_order: bool,
    pub transform: FeedbackTransform,
}

#[derive(Clone, Debug)]
pub struct FrameBuffer {
    width: usize,
    height: usize,
    pixels: Vec<Rgba>,
}

impl FrameBuffer {
    pub fn transparent(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![Rgba::TRANSPARENT; width * height],
        }
    }

    pub fn from_pixels(width: usize, height: usize, pixels: Vec<Rgba>) -> Option<Self> {
        (pixels.len() == width.checked_mul(height)?).then_some(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn pixels(&self) -> &[Rgba] {
        &self.pixels
    }
    pub fn byte_len(&self) -> usize {
        self.pixels.len() * std::mem::size_of::<Rgba>()
    }
    pub fn get(&self, x: usize, y: usize) -> Rgba {
        self.pixels[y * self.width + x]
    }

    /// One exact recurrence step. A missing source is transparent.
    pub fn step(&mut self, source: Option<&[Rgba]>, params: FeedbackStep) {
        if let Some(src) = source {
            assert_eq!(src.len(), self.pixels.len());
        }
        let width = self.width;
        self.step_with(
            |x, y| source.map_or(Rgba::TRANSPARENT, |src| src[y * width + x]),
            params,
        );
    }

    /// Variant that reads a host layer without copying its whole image first.
    pub fn step_with(
        &mut self,
        mut source: impl FnMut(usize, usize) -> Rgba,
        params: FeedbackStep,
    ) {
        if params.transform.is_identity() {
            for (i, old) in self.pixels.iter_mut().enumerate() {
                let src = source(i % self.width, i / self.width);
                *old = blend_step(
                    *old,
                    src,
                    params.decay,
                    params.mode,
                    params.opacity,
                    params.swap_order,
                );
            }
            return;
        }
        let previous = self.transformed(params.transform);
        let prev = previous.as_slice();
        let next = prev
            .iter()
            .enumerate()
            .map(|(i, &old)| {
                let src = source(i % self.width, i / self.width);
                blend_step(
                    old,
                    src,
                    params.decay,
                    params.mode,
                    params.opacity,
                    params.swap_order,
                )
            })
            .collect();
        self.pixels = next;
    }

    /// Incorporate a frame processed by an external feedback chain. The chain
    /// receives the *previous* state and must return the same image dimensions.
    /// This is the recurrence needed for an AE effect stack between two markers.
    pub fn step_with_processed(
        &mut self,
        processed_previous: &[Rgba],
        mut source: impl FnMut(usize, usize) -> Rgba,
        decay: [f32; 3],
        mode: BlendMode,
        opacity: f32,
    ) -> Result<(), &'static str> {
        if processed_previous.len() != self.pixels.len() {
            return Err("feedback chain dimensions changed");
        }
        for (i, &processed) in processed_previous.iter().enumerate() {
            let input = source(i % self.width, i / self.width);
            self.pixels[i] = blend_step(processed, input, decay, mode, opacity, false);
        }
        Ok(())
    }

    fn transformed(&self, transform: FeedbackTransform) -> Vec<Rgba> {
        let (sin, cos) = transform.rotation_degrees.to_radians().sin_cos();
        let scale = transform.scale.max(0.0001);
        let cx = (self.width as f32 - 1.0) * 0.5;
        let cy = (self.height as f32 - 1.0) * 0.5;
        let mut result = vec![Rgba::TRANSPARENT; self.pixels.len()];
        for y in 0..self.height {
            for x in 0..self.width {
                let dx = x as f32 - cx - transform.x;
                let dy = y as f32 - cy - transform.y;
                let sx = (cos * dx + sin * dy) / scale + cx;
                let sy = (-sin * dx + cos * dy) / scale + cy;
                result[y * self.width + x] = self.sample_bilinear(sx, sy);
            }
        }
        if transform.blur_radius == 0 {
            result
        } else {
            box_blur(
                &result,
                self.width,
                self.height,
                transform.blur_radius as usize,
            )
        }
    }

    fn sample_bilinear(&self, x: f32, y: f32) -> Rgba {
        if !x.is_finite() || !y.is_finite() {
            return Rgba::TRANSPARENT;
        }
        let x0 = x.floor() as isize;
        let y0 = y.floor() as isize;
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;
        let read = |xx: isize, yy: isize| {
            if xx < 0 || yy < 0 || xx >= self.width as isize || yy >= self.height as isize {
                Rgba::TRANSPARENT
            } else {
                self.get(xx as usize, yy as usize)
            }
        };
        let top = read(x0, y0).lerp(read(x0 + 1, y0), fx);
        let bottom = read(x0, y0 + 1).lerp(read(x0 + 1, y0 + 1), fx);
        top.lerp(bottom, fy)
    }
}

fn box_blur(input: &[Rgba], width: usize, height: usize, radius: usize) -> Vec<Rgba> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let mut horizontal = vec![Rgba::TRANSPARENT; input.len()];
    for y in 0..height {
        let mut sum = Rgba::TRANSPARENT;
        for x in 0..=radius.min(width - 1) {
            add(&mut sum, input[y * width + x]);
        }
        for x in 0..width {
            let count = (x + radius).min(width - 1) - x.saturating_sub(radius) + 1;
            horizontal[y * width + x] = divided(sum, count as f32);
            if x >= radius {
                subtract(&mut sum, input[y * width + x - radius]);
            }
            if x + radius + 1 < width {
                add(&mut sum, input[y * width + x + radius + 1]);
            }
        }
    }
    let mut out = vec![Rgba::TRANSPARENT; input.len()];
    for x in 0..width {
        let mut sum = Rgba::TRANSPARENT;
        for y in 0..=radius.min(height - 1) {
            add(&mut sum, horizontal[y * width + x]);
        }
        for y in 0..height {
            let count = (y + radius).min(height - 1) - y.saturating_sub(radius) + 1;
            out[y * width + x] = divided(sum, count as f32);
            if y >= radius {
                subtract(&mut sum, horizontal[(y - radius) * width + x]);
            }
            if y + radius + 1 < height {
                add(&mut sum, horizontal[(y + radius + 1) * width + x]);
            }
        }
    }
    out
}

#[inline]
fn add(sum: &mut Rgba, v: Rgba) {
    sum.r += v.r;
    sum.g += v.g;
    sum.b += v.b;
    sum.a += v.a;
}
#[inline]
fn subtract(sum: &mut Rgba, v: Rgba) {
    sum.r -= v.r;
    sum.g -= v.g;
    sum.b -= v.b;
    sum.a -= v.a;
}
#[inline]
fn divided(v: Rgba, d: f32) -> Rgba {
    Rgba::new(v.r / d, v.g / d, v.b / d, v.a / d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn add_step(transform: FeedbackTransform) -> FeedbackStep {
        FeedbackStep {
            decay: [1.0; 3],
            mode: BlendMode::Add,
            opacity: 1.0,
            swap_order: false,
            transform,
        }
    }

    #[test]
    fn unity_decay_keeps_an_impulse_past_the_old_window_limit() {
        let mut state = FrameBuffer::transparent(1, 1);
        state.step(
            Some(&[Rgba::new(1.0, 1.0, 1.0, 1.0)]),
            add_step(Default::default()),
        );
        for _ in 0..500 {
            state.step(None, add_step(Default::default()));
        }
        assert_eq!(state.get(0, 0).r, 1.0);
    }

    #[test]
    fn animated_decay_applies_at_its_own_frame() {
        let mut state = FrameBuffer::transparent(1, 1);
        state.step(
            Some(&[Rgba::new(1.0, 0.0, 0.0, 1.0)]),
            add_step(Default::default()),
        );
        state.step(None, add_step(Default::default()));
        state.step(
            None,
            FeedbackStep {
                decay: [0.5; 3],
                ..add_step(Default::default())
            },
        );
        assert_eq!(state.get(0, 0).r, 0.5);
    }

    #[test]
    fn translation_feeds_the_previous_frame_back() {
        let mut state = FrameBuffer::transparent(5, 1);
        let mut input = vec![Rgba::TRANSPARENT; 5];
        input[1] = Rgba::new(1.0, 0.0, 0.0, 1.0);
        state.step(Some(&input), add_step(Default::default()));
        state.step(
            None,
            add_step(FeedbackTransform {
                x: 1.0,
                ..Default::default()
            }),
        );
        assert_eq!(state.get(2, 0).r, 1.0);
        assert_eq!(state.get(1, 0).r, 0.0);
    }

    #[test]
    fn blur_spreads_previous_state_with_edge_normalization() {
        let input = [
            Rgba::TRANSPARENT,
            Rgba::new(1.0, 0.0, 0.0, 1.0),
            Rgba::TRANSPARENT,
        ];
        let blurred = box_blur(&input, 3, 1, 1);
        assert!((blurred[0].r - 0.5).abs() < 1e-6);
        assert!((blurred[1].r - 1.0 / 3.0).abs() < 1e-6);
        assert!((blurred[2].r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn checkpoints_match_cold_render_in_any_order() {
        fn render(frame: i32, cache: &mut BTreeMap<i32, FrameBuffer>) -> FrameBuffer {
            let (start, mut state) = cache
                .range(..=frame)
                .next_back()
                .map_or((0, FrameBuffer::transparent(3, 1)), |(&f, state)| {
                    (f + 1, state.clone())
                });
            for t in start..=frame {
                let mut src = [Rgba::TRANSPARENT; 3];
                if t == 0 || t == 7 {
                    src[0] = Rgba::new(1.0, 0.4, 0.1, 1.0);
                }
                let step = FeedbackStep {
                    decay: [if t < 12 { 1.0 } else { 0.91 }; 3],
                    mode: BlendMode::Add,
                    opacity: 1.0,
                    swap_order: false,
                    transform: FeedbackTransform {
                        x: if t % 3 == 0 { 1.0 } else { 0.0 },
                        ..Default::default()
                    },
                };
                state.step(Some(&src), step);
                if t % 4 == 0 || t == frame {
                    cache.insert(t, state.clone());
                }
            }
            state
        }

        let expected: Vec<_> = (0..40).map(|f| render(f, &mut BTreeMap::new())).collect();
        let mut cache = BTreeMap::new();
        let mut order: Vec<i32> = (0..40).collect();
        order.sort_by_key(|f| (f * 17) % 40);
        for f in order {
            let actual = render(f, &mut cache);
            assert_eq!(expected[f as usize].pixels(), actual.pixels(), "frame {f}");
        }
    }

    #[test]
    fn external_chain_result_becomes_the_next_feedback_state() {
        let mut state = FrameBuffer::transparent(3, 1);
        state.step(
            Some(&[
                Rgba::new(1.0, 0.0, 0.0, 1.0),
                Rgba::TRANSPARENT,
                Rgba::TRANSPARENT,
            ]),
            add_step(Default::default()),
        );
        let processed = [Rgba::TRANSPARENT, state.get(0, 0), Rgba::TRANSPARENT];
        state
            .step_with_processed(
                &processed,
                |_, _| Rgba::TRANSPARENT,
                [1.0; 3],
                BlendMode::Add,
                1.0,
            )
            .unwrap();
        assert_eq!(state.get(1, 0).r, 1.0);
        assert_eq!(state.get(0, 0).r, 0.0);
    }
}
