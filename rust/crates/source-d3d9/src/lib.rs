//! Direct3D 9 on Metal.
//!
//! `shaderapidx9` issues the Direct3D 9 calls it always has and this crate
//! turns them into Metal, in the place ToGL turned them into OpenGL. The
//! design and the conventions the modules share are in
//! `docs/rust-port/d3d9-metal.md`.

pub mod format;
pub mod state;
pub mod translate;

#[cfg(target_os = "macos")]
pub mod device;
#[cfg(target_os = "macos")]
pub mod mtl;
#[cfg(target_os = "macos")]
pub mod objc;
