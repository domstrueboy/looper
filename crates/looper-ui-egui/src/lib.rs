//! The screens, drawn with egui.
//!
//! A leaf: each screen reads its state and returns `Action`s, and the
//! only way it reaches the rest of the app is by being called. Nothing
//! here decides anything - which is what makes swapping toolkit, or
//! adding a second look, a matter of writing another crate beside this
//! one rather than touching what the app does.
//!
//! `egui` appears here and in `looper-app`'s shell, nowhere else.

pub mod indicator;
pub mod looper;
pub mod settings;
