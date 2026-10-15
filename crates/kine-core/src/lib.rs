//! Ядро kine: дерево сцены из команд JS, разрешение стилей, раскладка, текст и
//! представления для агента (дамп, inspect, диагностики).

pub mod diag;
pub mod dump;
pub mod layout;
pub mod look;
pub mod must;
pub mod scene;
pub mod text;

pub use layout::{Frame, FrameNode, Rect, build_frame, build_frame_with};
pub use must::{EvalOptions, Mode, evaluate};
pub use scene::Scene;
pub use text::TextEngine;
