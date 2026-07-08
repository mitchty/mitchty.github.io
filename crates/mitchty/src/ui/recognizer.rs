use bevy::prelude::*;
use bevy_egui::egui;

use crate::ai::infer::CanvasData;

/// Rasterization grid size used when converting strokes to pixels for inference.
///
/// Will go away when I ditch mnist stuff entirely, or maybe I should figure out
/// a more dynaic way. As is tradition future mitch problems sucker!!!!!
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RasterSize {
    #[default]
    S128,
}

impl RasterSize {
    /// Side length in pixels.
    pub fn pixels(self) -> usize {
        match self {
            Self::S128 => 128,
        }
    }

    /// Display label used for gooey.
    #[cfg(dev_build)]
    pub fn label(self) -> &'static str {
        match self {
            Self::S128 => "128x128",
        }
    }
}

/// Baseline brush radius in raster-grid units at 28x28 (for now ref ^^^^)
pub const BASE_BRUSH_R: f32 = 1.4;

/// All drawing state for the Recognizer window.
#[derive(Resource)]
pub struct RecognizerState {
    /// Completed strokes, each strokes just a list of points
    pub strokes: Vec<Vec<egui::Pos2>>,
    /// Strokes that have been undone and can be redone.
    pub redo_stack: Vec<Vec<egui::Pos2>>,
    /// The stroke currently being drawn.
    pub current_stroke: Option<Vec<egui::Pos2>>,
    /// When true, draw a red bounding-box overlay around the cropped region
    /// that is actually sent to the classifier so I can eyeball if what I'm
    /// doing is sane.
    pub debug_bbox: bool,
    /// Rasterisation grid size, debug build only feature for now.
    pub raster_size: RasterSize,
    /// Multiplier applied to the base brush radius for stroke size, rough range
    /// [0.5, 4.0] for now. Not pretty nor attempted to make pretty yet.
    pub stroke_scale: f32,
}

impl Default for RecognizerState {
    fn default() -> Self {
        Self {
            strokes: Vec::new(),
            redo_stack: Vec::new(),
            current_stroke: None,
            debug_bbox: false,
            raster_size: RasterSize::default(),
            stroke_scale: 1.0,
        }
    }
}

/// The result of running the inference engine against a canvas snapshot.
///
/// Stored as a Bevy [`Resource`]. Updated whenever a stroke is committed or
/// the canvas is cleared. `matches` is sorted descending by confidence.
#[derive(Resource, Default)]
pub struct InferenceResult {
    /// The rasterized canvas that produced this result, or `None` if the
    /// canvas has never been run through inference.
    #[allow(dead_code)]
    pub canvas: Option<CanvasData>,
    /// `(class_index, confidence)` pairs, descending by confidence.
    pub matches: Vec<(usize, f32)>,
    /// Tight bounding box of the user's drawing in **canvas-local coordinates**
    ///
    /// `(min_x, min_y, max_x, max_y)`. `None` when the canvas is blank.
    pub bbox: Option<(f32, f32, f32, f32)>,
}

impl RecognizerState {
    /// Commit the in-progress stroke to the finished list and clear the redo
    /// stack, since a new stroke invalidates the redo history.
    pub fn commit_stroke(&mut self) {
        if let Some(stroke) = self.current_stroke.take()
            && stroke.len() >= 2
        {
            self.redo_stack.clear();
            self.strokes.push(stroke);
        }
    }

    pub fn undo(&mut self) {
        self.current_stroke = None;
        if let Some(stroke) = self.strokes.pop() {
            self.redo_stack.push(stroke);
        }
    }

    pub fn redo(&mut self) {
        if let Some(stroke) = self.redo_stack.pop() {
            self.strokes.push(stroke);
        }
    }

    pub fn clear(&mut self) {
        self.strokes.clear();
        self.redo_stack.clear();
        self.current_stroke = None;
    }
}
