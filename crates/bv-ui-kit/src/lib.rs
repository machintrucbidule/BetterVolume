//! BetterVolume design system.
//!
//! The only place where colours, metrics, typography, motion curves and control rendering
//! may live (NFR-04, `docs/ARCHITECTURE.md` §4). Feature code asks this crate for a themed
//! control and never draws or hard-codes a colour, size, duration or easing itself. This
//! crate, in turn, knows nothing about audio or features.
//!
//! - [`theme`]: from the Windows appearance preferences to resolved colours.
//! - [`tokens`], [`motion`]: metrics, typography, durations and easing curves.
//! - [`icons`]: the Windows icon-font glyphs and the volume icon steps.
//! - [`controls`]: the control renderers (tray icon, context menu, flyout panel).
//! - [`backend`]: Direct2D / DirectWrite / DirectComposition rendering.
//!
//! `unsafe` is denied here and allowed only inside the rendering backend module.
#![deny(unsafe_code)]

pub mod access;
pub mod backend;
mod color;
pub mod controls;
mod geometry;
pub mod icons;
pub mod motion;
pub mod path;
pub mod theme;
pub mod tokens;

pub use color::Color;
pub use geometry::{Point, Rect, Size};
pub use theme::{Theme, ThemeInputs, ThemeMode};
