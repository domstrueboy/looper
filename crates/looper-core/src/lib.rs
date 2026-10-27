//! What the looper does, with nothing about the machine it does it on.
//!
//! No `cpal` and no `egui`: the device layer is `looper-hal` and the
//! screens are a UI crate, so what is left here is the pedal itself -
//! the state machine, the layer stack, the audio path, the settings and
//! the saved loop. Swapping either end doesn't reach in here, and these
//! tests run anywhere without an audio SDK to build against.

pub mod action;
pub mod audio;
pub mod config;
pub mod input;
pub mod loop_mirror;
pub mod looper;
pub mod preroll;
pub mod sample;
pub mod settings;
pub mod state_machine;
pub mod tap_tempo;
pub mod tempo;
pub mod wav;
