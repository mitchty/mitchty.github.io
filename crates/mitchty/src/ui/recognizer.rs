use bevy::prelude::*;
use bevy_egui::egui;

use crate::ai::infer::CanvasData;

/// All drawing state for the Recognizer window.
#[derive(Resource, Default)]
pub struct RecognizerState {
    /// Completed strokes – each stroke is an ordered list of points.
    pub strokes: Vec<Vec<egui::Pos2>>,
    /// Strokes that have been undone and can be redone.
    pub redo_stack: Vec<Vec<egui::Pos2>>,
    /// The stroke currently being drawn (pointer held down).
    pub current_stroke: Option<Vec<egui::Pos2>>,
}

/// The result of running the inference engine against a canvas snapshot.
///
/// Stored as a Bevy [`Resource`].  Updated whenever a stroke is committed or
/// the canvas is cleared.  `matches` is sorted descending by confidence.
#[derive(Resource, Default)]
pub struct InferenceResult {
    /// The rasterised canvas that produced this result, or `None` if the
    /// canvas has never been run through inference.
    #[allow(dead_code)]
    pub canvas: Option<CanvasData>,
    /// `(class_index, confidence)` pairs, descending by confidence.
    pub matches: Vec<(usize, f32)>,
}

impl RecognizerState {
    /// Commit the in-progress stroke (if any) to the finished list and clear
    /// the redo stack, since a new stroke invalidates the redo history.
    pub fn commit_stroke(&mut self) {
        if let Some(stroke) = self.current_stroke.take()
            && stroke.len() >= 2
        {
            self.redo_stack.clear();
            self.strokes.push(stroke);
        }
    }

    pub fn undo(&mut self) {
        // Cancel any in-progress stroke first.
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
