//! Ядро kine: дерево сцены из команд JS, разрешение стилей, раскладка, текст и
//! представления для агента (дамп, inspect, диагностики).

pub mod diag;
pub mod display;
pub mod dump;
pub mod layout;
pub mod look;
pub mod motion;
pub mod must;
pub mod scene;
pub mod timeline;
pub mod text;

pub use display::DisplayList;
pub use layout::{Frame, FrameNode, Rect, build_frame, build_frame_with};
pub use motion::{Animated, MotionState};
pub use must::{EvalOptions, Mode, evaluate, evaluate_at};
pub use scene::Scene;
pub use text::TextEngine;
