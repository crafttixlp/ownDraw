//! OwnDraw-Kern: Board-Modell, Formerkennung, Rendering, Ablage und Export.
#![allow(clippy::type_complexity, clippy::neg_cmp_op_on_partial_ord, clippy::too_many_arguments, clippy::needless_range_loop, clippy::manual_div_ceil, clippy::unnecessary_sort_by)]

pub mod board;
pub mod calc;
pub mod canvas;
pub mod collab;
pub mod doc;
pub mod export;
pub mod features;
pub mod geom;
pub mod hw;
pub mod icons;
pub mod item;
pub mod laser;
pub mod layer;
pub mod math;
pub mod migrate;
pub mod pages;
pub mod recognizer;
pub mod store;
pub mod text;
pub mod xopp;

pub use tiny_skia;
