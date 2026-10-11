#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod animation;
mod audio;
mod binary;
mod generation;
mod mesh;

pub use animation::{
    clip_alias, form_clips, form_gear, form_graph, form_marker, form_mesh, form_state, form_trace,
    form_transition, form_vfx, forms, gear_toggle, vfx_markers,
};
pub use audio::{form_sound, sound_bank};
pub use generation::{builder, client_data, generator};
pub use mesh::{mesh_merge, skeleton, skinned_mesh};

pub mod error;
