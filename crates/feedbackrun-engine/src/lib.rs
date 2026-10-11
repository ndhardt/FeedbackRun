//! Evaluation core. No After Effects dependency, no hidden state.
//!
//! ```text
//! state[t] = Blend( decay * state[t-1], src[t] )
//! ```

pub mod blend;
pub mod eval;
pub mod feedback;
pub mod post;
pub mod window;

pub use blend::{BlendMode, Rgba, blend_step};
pub use eval::{StateParams, eval_state};
pub use feedback::{FeedbackStep, FeedbackTransform, FrameBuffer};
pub use post::{PostParams, ShowMode, apply_post};
pub use window::{PreAnchorMode, Window, truncation_epsilon, window_for};
