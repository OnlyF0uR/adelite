//! Adelite's interface toolkit. No web view and no UI framework: winit
//! provides the window, wgpu draws every pixel through a single instanced
//! SDF pipeline ([`gpu`], `shader.wgsl`), and the interface is immediate
//! mode ([`ui`], [`widgets`]), redrawn only when something changes.
//!
//! An app on the rail (chat, and whatever comes after it) is a [`Module`]:
//! the shell knows nothing else about it.

pub mod atlas;
pub mod editor;
pub mod font;
pub mod form;
pub mod gpu;
pub mod image;
pub mod log;
pub mod logo;
pub mod module;
pub mod paint;
pub mod platform;
pub mod raster;
#[cfg(any(test, feature = "offscreen"))]
pub mod snapshot;
pub mod text;
pub mod theme;
pub mod titlebar;
pub mod ui;
pub mod widgets;

pub use module::{Context, Module, Request, Sound, Waker};
