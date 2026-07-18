#[cfg(all(dev_build, not(target_arch = "wasm32")))]
use burn::record::CompactRecorder;
use burn::{
    backend::NdArray,
    nn::{
        BatchNorm, BatchNormConfig, Dropout, DropoutConfig, Relu,
        conv::{Conv2d, Conv2dConfig},
        pool::{AdaptiveAvgPool2d, AdaptiveAvgPool2dConfig},
    },
    prelude::*,
    record::{HalfPrecisionSettings, NamedMpkBytesRecorder, Recorder},
};

use bevy_egui::egui;

use crate::ui::RecognizerState;
use crate::ui::recognizer::{BASE_BRUSH_R, RasterSize};

#[derive(Clone, Debug)]
pub struct CanvasData {
    pub pixels: Vec<f32>,
    // Clamped to 128 for now until I get mnist working again if ever.
    /// Matches the `raster_size` that was active when `rasterize()` was called.
    pub size: usize,
}

impl CanvasData {
    /// Return the pixel at `(row, col)`.
    #[allow(dead_code)]
    pub fn pixel(&self, row: usize, col: usize) -> f32 {
        self.pixels[row * self.size + col]
    }
}

// TODO: There is a lot of shared code I need to refactor into a library at some point.
#[derive(Config, Debug)]
struct ModelConfig {
    num_classes: usize,
    hidden_size: usize,
    #[config(default = "0.5")]
    dropout: f64,
    /// Output channels after conv1. conv2 doubles this, conv3 doubles again.
    /// Default 32 gives 32->64->128, matching ma's ModelConfig default.
    #[config(default = "32")]
    conv_channels: usize,
    /// Number of input channels, I'm testing out training with 3 now for
    /// greyscale, otsu, sauvola but the results are absolute ass.
    ///
    /// Will likely need to augment data and retrain. Default is 3 for now cause
    /// I'm training only that.
    #[config(default = "3")]
    input_channels: usize,
}

#[derive(Module, Debug)]
struct Model<B: Backend> {
    conv1: Conv2d<B>,
    bn1: BatchNorm<B>,
    conv2: Conv2d<B>,
    bn2: BatchNorm<B>,
    conv3: Conv2d<B>,
    bn3: BatchNorm<B>,
    pool: AdaptiveAvgPool2d,
    dropout: Dropout,
    linear1: burn::nn::Linear<B>,
    linear2: burn::nn::Linear<B>,
    activation: Relu,
    input_channels: usize,
}

impl ModelConfig {
    /// Architecture mirrors ma's ModelConfig::init exactly:
    ///   conv1 (C_in -> C, 3x3, pad=Same)   -> BN -> ReLU
    ///   conv2 (C -> 2C, 3x3, pad=Same)     -> BN -> ReLU -> Dropout
    ///   conv3 (2C -> 4C, 3x3, pad=Same)    -> BN -> ReLU -> Dropout
    ///   AdaptiveAvgPool2d([4, 4])          -> [B, 4C, 4, 4]
    ///   Linear(4C*16 -> hidden_size)       -> Dropout -> ReLU
    ///   Linear(hidden_size -> num_classes)
    fn init<B: Backend>(&self, device: &B::Device) -> Model<B> {
        let c = self.conv_channels;
        Model {
            conv1: Conv2dConfig::new([self.input_channels, c], [3, 3])
                .with_padding(burn::nn::PaddingConfig2d::Same)
                .init(device),
            bn1: BatchNormConfig::new(c).init(device),
            conv2: Conv2dConfig::new([c, c * 2], [3, 3])
                .with_padding(burn::nn::PaddingConfig2d::Same)
                .init(device),
            bn2: BatchNormConfig::new(c * 2).init(device),
            conv3: Conv2dConfig::new([c * 2, c * 4], [3, 3])
                .with_padding(burn::nn::PaddingConfig2d::Same)
                .init(device),
            bn3: BatchNormConfig::new(c * 4).init(device),
            pool: AdaptiveAvgPool2dConfig::new([4, 4]).init(),
            activation: Relu::new(),
            linear1: burn::nn::LinearConfig::new(c * 4 * 4 * 4, self.hidden_size).init(device),
            linear2: burn::nn::LinearConfig::new(self.hidden_size, self.num_classes).init(device),
            dropout: DropoutConfig::new(self.dropout).init(),
            input_channels: self.input_channels,
        }
    }
}

impl<B: Backend> Model<B> {
    /// Forward pass. Input: `[batch, channels, height, width]` -> output: `[batch, num_classes]`.
    fn forward(&self, images: Tensor<B, 4>) -> Tensor<B, 2> {
        let [batch_size, _channels, _height, _width] = images.dims();
        // Use images directly - channel dimension already added by caller
        let x = images;

        // Block 1
        let x = self.conv1.forward(x);
        let x = self.bn1.forward(x);
        let x = self.activation.forward(x);

        // Block 2
        let x = self.conv2.forward(x);
        let x = self.bn2.forward(x);
        let x = self.activation.forward(x);
        let x = self.dropout.forward(x);

        // Block 3
        let x = self.conv3.forward(x);
        let x = self.bn3.forward(x);
        let x = self.activation.forward(x);
        let x = self.dropout.forward(x);

        // Pool + flatten: [B, 4C, 4, 4] -> [B, 4C*16]
        let x = self.pool.forward(x);
        let features = x.dims()[1] * x.dims()[2] * x.dims()[3];
        let x = x.reshape([batch_size, features]);

        // Classifier head
        let x = self.linear1.forward(x);
        let x = self.dropout.forward(x);
        let x = self.activation.forward(x);
        self.linear2.forward(x)
    }
}

#[derive(Debug)]
struct SavedModelConfig {
    num_classes: usize,
    hidden_size: usize,
    dropout: f64,
    conv_channels: usize,
    input_channels: usize,
}

#[derive(Debug)]
struct SavedTrainingConfig {
    model: SavedModelConfig,
    /// Normalisation mean saved by `ma`'s TrainingConfig.
    norm_mean: f64,
    /// Normalisation std saved by `ma`'s TrainingConfig.
    norm_std: f64,
    /// Class-index -> Unicode character mapping embedded at training time from
    /// `ma convert`'s classmap.json. Empty for models trained without one.
    class_map: Vec<char>,
}

impl SavedTrainingConfig {
    fn from_json(text: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let model = &v["model"];

        // Parse class_map: array of single-char strings, e.g. ["あ","い",...]
        let class_map: Vec<char> = v["class_map"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str()?.chars().next())
                    .collect()
            })
            .unwrap_or_default();

        Some(Self {
            model: SavedModelConfig {
                num_classes: model["num_classes"].as_u64()? as usize,
                hidden_size: model["hidden_size"].as_u64()? as usize,
                dropout: model["dropout"].as_f64().unwrap_or(0.5),
                conv_channels: model["conv_channels"].as_u64().unwrap_or(32) as usize,
                input_channels: model["channels"].as_u64().unwrap_or(1) as usize,
            },
            norm_mean: v["norm_mean"].as_f64().unwrap_or(0.1793),
            norm_std: v["norm_std"].as_f64().unwrap_or(0.3416),
            class_map,
        })
    }
}

type NdArrayBackend = NdArray<f32>;

/// Loaded classifier model ready for single-image inference.
///
/// Stored as a Bevy `NonSend` resource because `NdArray` tensors are `!Send`.
pub struct InferenceEngine {
    model: Model<NdArrayBackend>,
    num_classes: usize,
    /// Normalisation mean loaded from the model's config.json.
    norm_mean: f32,
    /// Normalisation std loaded from the model's config.json.
    norm_std: f32,
    /// Class-index -> Unicode character map loaded from config.json.
    /// Empty for models trained without a classmap; `char_for_class` falls
    /// back to `k49_char` in that case for backwards compatibility.
    class_map: Vec<char>,
}

impl InferenceEngine {
    /// Load a trained model from `artifact_dir`.
    ///
    /// Expects:
    /// - `{artifact_dir}/config.json` - serialised `TrainingConfig`
    /// - `{artifact_dir}/model.mpk`   - `CompactRecorder` weights
    ///
    /// Returns `None` if either file is missing or fails to parse/load.
    #[cfg(all(dev_build, not(target_arch = "wasm32")))]
    pub fn load(artifact_dir: &str) -> Option<Self> {
        let config_path = format!("{artifact_dir}/config.json");
        let model_path = format!("{artifact_dir}/model");

        // Deserialise just the fields we need from the saved config.
        let config_text = std::fs::read_to_string(&config_path)
            .map_err(|e| bevy::log::warn!("InferenceEngine: cannot read {config_path}: {e}"))
            .ok()?;
        let saved: SavedTrainingConfig =
            SavedTrainingConfig::from_json(&config_text).or_else(|| {
                bevy::log::warn!("InferenceEngine: cannot parse config at {config_path}");
                None
            })?;

        let device = Default::default();
        let model_config = ModelConfig {
            num_classes: saved.model.num_classes,
            hidden_size: saved.model.hidden_size,
            dropout: saved.model.dropout,
            conv_channels: saved.model.conv_channels,
            input_channels: saved.model.input_channels,
        };

        let record = CompactRecorder::new()
            .load(model_path.into(), &device)
            .map_err(|e| bevy::log::warn!("InferenceEngine: cannot load model weights: {e}"))
            .ok()?;

        let model = model_config
            .init::<NdArrayBackend>(&device)
            .load_record(record);

        bevy::log::info!(
            "InferenceEngine loaded from {artifact_dir} (num_classes={}, norm={:.4}/{:.4}, classmap={})",
            saved.model.num_classes,
            saved.norm_mean,
            saved.norm_std,
            if saved.class_map.is_empty() {
                "none".to_owned()
            } else {
                format!("{} classes", saved.class_map.len())
            }
        );

        Some(Self {
            model,
            num_classes: saved.model.num_classes,
            norm_mean: saved.norm_mean as f32,
            norm_std: saved.norm_std as f32,
            class_map: saved.class_map,
        })
    }

    /// Load a trained model from embedded byte slices.
    ///
    /// `config_bytes` should be the raw JSON produced by `ma`'s `TrainingConfig`.
    /// `model_bytes` should be the raw `CompactRecorder` (Named MessagePack) weights.
    ///
    /// This is the preferred load path for WASM and for shipping a default model
    /// compiled directly into the binary via `include_bytes!`.
    ///
    /// Future Mitch gets to figure out the harder problem of making this dynamic.
    pub fn from_embedded(config_bytes: &[u8], model_bytes: &[u8]) -> Option<Self> {
        let config_text = core::str::from_utf8(config_bytes)
            .map_err(|e| {
                bevy::log::warn!("InferenceEngine: embedded config is not valid UTF-8: {e}")
            })
            .ok()?;

        let saved: SavedTrainingConfig =
            SavedTrainingConfig::from_json(config_text).or_else(|| {
                bevy::log::warn!("InferenceEngine: cannot parse embedded config JSON");
                None
            })?;

        let device = Default::default();
        let model_config = ModelConfig {
            num_classes: saved.model.num_classes,
            hidden_size: saved.model.hidden_size,
            dropout: saved.model.dropout,
            conv_channels: saved.model.conv_channels,
            input_channels: saved.model.input_channels,
        };

        let record = NamedMpkBytesRecorder::<HalfPrecisionSettings>::default()
            .load(model_bytes.to_vec(), &device)
            .map_err(|e| {
                bevy::log::warn!("InferenceEngine: cannot load embedded model weights: {e}")
            })
            .ok()?;

        let model = model_config
            .init::<NdArrayBackend>(&device)
            .load_record(record);

        bevy::log::info!(
            "InferenceEngine loaded from embedded bytes (num_classes={}, norm={:.4}/{:.4}, classmap={})",
            saved.model.num_classes,
            saved.norm_mean,
            saved.norm_std,
            if saved.class_map.is_empty() {
                "none".to_owned()
            } else {
                format!("{} classes", saved.class_map.len())
            }
        );

        Some(Self {
            model,
            num_classes: saved.model.num_classes,
            norm_mean: saved.norm_mean as f32,
            norm_std: saved.norm_std as f32,
            class_map: saved.class_map,
        })
    }

    /// Rasterize the current canvas state into a square pixel grid and return
    /// the tight bounding box of the drawing in canvas-local coordinates for
    /// classification.
    ///
    /// Note this is where the padding/detection for the bounding box gets done
    /// too. The midpoint of a drawing is used to ensure the bounding box is
    /// square even for a vertical line and both sides are roughly equal. It
    /// doesn't need to be perfect having a lhs of 49 and rhs of 50 is fine with
    /// a 1px line.
    pub fn rasterize(state: &RecognizerState) -> (CanvasData, Option<(f32, f32, f32, f32)>) {
        // Border to leave on each side, expressed as a fraction of the grid.
        // 128x128 grids get a ~9px border
        let sz = state.raster_size.pixels();
        let padding: f32 = 2.0 * sz as f32 / RasterSize::S128.pixels() as f32;
        // Brush radius: base value scaled for grid size, then multiplied by the
        // user-controlled stroke_scale so thickness can be tuned at runtime to
        // see how badly I trained the model or not.
        let brush_r: f32 =
            BASE_BRUSH_R * sz as f32 / RasterSize::S128.pixels() as f32 * state.stroke_scale;

        let mut grid = vec![0.0f32; sz * sz];

        let all_points: Vec<egui::Pos2> = state
            .strokes
            .iter()
            .chain(state.current_stroke.iter())
            .flat_map(|s| s.iter().copied())
            .collect();

        if all_points.is_empty() {
            return (
                CanvasData {
                    pixels: grid,
                    size: sz,
                },
                None,
            );
        }

        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for p in &all_points {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }

        // Guard against single-point drawings that are useless.
        let bbox_w = (max_x - min_x).max(1.0);
        let bbox_h = (max_y - min_y).max(1.0);

        // Find the bbox for the drawing, use the longer of h or w. Then pad
        // from the remaining midpoint to the longer of the two. so that the
        // classifier data matches the training data more correctly.
        let bbox_side = bbox_w.max(bbox_h);
        let cx = (min_x + max_x) / 2.0;
        let cy = (min_y + max_y) / 2.0;
        let sq_min_x = cx - bbox_side / 2.0;
        let sq_min_y = cy - bbox_side / 2.0;
        let sq_max_x = cx + bbox_side / 2.0;
        let sq_max_y = cy + bbox_side / 2.0;

        let usable = sz as f32 - 2.0 * padding;

        let scale = usable / bbox_side;

        // Map a canvas-local point into grid coordinates.
        let to_grid = |p: egui::Pos2| {
            let gx = (p.x - sq_min_x) * scale + padding;
            let gy = (p.y - sq_min_y) * scale + padding;
            (gx, gy)
        };

        let all_strokes = state.strokes.iter().chain(state.current_stroke.iter());

        for stroke in all_strokes {
            let stroke: &Vec<egui::Pos2> = stroke;
            // Single-point dot, I should just ignore these at some point maybe.
            // TODO: future me brain on this
            if stroke.len() == 1 {
                let (gx, gy) = to_grid(stroke[0]);
                paint_point(&mut grid, sz, gx, gy, brush_r);
            }
            for pair in stroke.windows(2) {
                let (ax, ay) = to_grid(pair[0]);
                let (bx, by) = to_grid(pair[1]);
                draw_segment(&mut grid, sz, ax, ay, bx, by, brush_r);
            }
        }

        // Clamp to [0, 1].
        for v in &mut grid {
            *v = v.clamp(0.0, 1.0);
        }

        // Return the calculated bbox data this is what the classifier "sees"
        // mapped back into canvas-local coordinates, what the debug overlay
        // draws.
        (
            CanvasData {
                pixels: grid,
                size: sz,
            },
            Some((sq_min_x, sq_min_y, sq_max_x, sq_max_y)),
        )
    }

    /// Return the Unicode character for `class_idx` using the classmap
    /// embedded in `config.json` at training time.
    pub fn char_for_class(&self, class_idx: usize) -> Option<char> {
        self.class_map.get(class_idx).copied()
    }

    /// Run the classifier on `canvas` and return class confidences.
    ///
    /// Returns a `Vec<(class_index, confidence)>` sorted by confidence
    /// **descending**. `confidence` is in `[0.0, 1.0]`.
    ///
    /// Returns an empty vec if the canvas is blank (all zeros).
    pub fn run(&self, canvas: &CanvasData) -> Vec<(usize, f32)> {
        // Skip blank canvases - avoids noisy results for an empty drawing.
        if canvas.pixels.iter().all(|&v| v == 0.0) {
            return vec![];
        }

        let sz = canvas.size;
        let device: Device<NdArrayBackend> = Default::default();

        // Normalize with the stats recorded in config.json at training time.
        let mean = self.norm_mean;
        let std = self.norm_std;
        let normalized: Vec<f32> = canvas.pixels.iter().map(|&v| (v - mean) / std).collect();

        // Always build 4D tensor [1, channels, H, W] for the model for now.
        // If model expects multiple channels but we have grayscale, duplicate
        // the channel so it kinda matches how things were trained.
        let num_channels = self.model.input_channels;
        let data = if num_channels == 1 {
            // TODO: here for if I ever need to revert back to this or not
            burn::tensor::TensorData::new(normalized, [1, 1, sz, sz])
        } else {
            let total_elements = normalized.len() * num_channels;
            let mut multi_channel = Vec::with_capacity(total_elements);
            for pixel in &normalized {
                for _ in 0..num_channels {
                    multi_channel.push(*pixel);
                }
            }
            burn::tensor::TensorData::new(multi_channel, [1, num_channels, sz, sz])
        };
        let tensor = Tensor::<NdArrayBackend, 4>::from_data(data, &device);

        // With NdArray no autodiff, Dropout checks B::ad_enabled() and is a
        // no-op when false - so we can call forward directly without .valid().
        let logits = self.model.forward(tensor);

        // Softmax over the class dimension.
        let probs = burn::tensor::activation::softmax(logits, 1);

        // Extract to Vec<f32>.
        let probs_data = probs.into_data().convert::<f32>();
        let values: Vec<f32> = probs_data.as_slice::<f32>().unwrap_or(&[]).to_vec();

        // Build (class, confidence) pairs and sort descending.
        let mut ranked: Vec<(usize, f32)> = values
            .into_iter()
            .enumerate()
            .take(self.num_classes)
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        ranked
    }
}

//TODO: Can I abuse my wgpu hacks to use shaders to do this instead? Make an svg shader maybe?

/// Rasterise a line segment from `(ax, ay)` to `(bx, by)` into `grid`.
///
/// `radius` is in grid-pixel units. Points are spaced every 0.5 grid-pixels
/// along the segment so there are no gaps even for diagonal strokes.
fn draw_segment(grid: &mut [f32], sz: usize, ax: f32, ay: f32, bx: f32, by: f32, radius: f32) {
    let dx = bx - ax;
    let dy = by - ay;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-3 {
        paint_point(grid, sz, ax, ay, radius);
        return;
    }

    // Step every half grid-pixel so diagonal segments have no gaps.
    let steps = (len * 2.0).ceil() as usize;
    let inv = 1.0 / steps as f32;
    for i in 0..=steps {
        let t = i as f32 * inv;
        paint_point(grid, sz, ax + dx * t, ay + dy * t, radius);
    }
}

/// Accumulate ink/black into all pixels within `radius` grid-pixels of `(px, py)`.
///
/// Weight falls off linearly from 1.0 at the center to 0.0 at `radius`.
fn paint_point(grid: &mut [f32], sz: usize, px: f32, py: f32, radius: f32) {
    let r_ceil = radius.ceil() as isize;
    let x0 = ((px - radius).floor() as isize).max(0) as usize;
    let x1 = ((px + radius).ceil() as isize).min(sz as isize - 1).max(0) as usize;
    let y0 = ((py - radius).floor() as isize).max(0) as usize;
    let y1 = ((py + radius).ceil() as isize).min(sz as isize - 1).max(0) as usize;
    let _ = r_ceil; // only used for bounds computation above

    for row in y0..=y1 {
        for col in x0..=x1 {
            let dist = ((col as f32 - px).powi(2) + (row as f32 - py).powi(2)).sqrt();
            if dist < radius {
                let weight = 1.0 - dist / radius;
                grid[row * sz + col] += weight;
            }
        }
    }
}
