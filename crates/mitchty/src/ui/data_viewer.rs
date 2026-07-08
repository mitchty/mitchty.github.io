//! Data viewer window to inspect npz training datasets visually cause some of
//! these datasets are... not obviously bad until you look at them.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use lib::img::{AutoMethod, ImagePipelineConfig, ThresholdMode, run_pipeline};

// Native-only egui file dialog for folder selection; not compiled for WASM cause no fs
#[cfg(not(target_arch = "wasm32"))]
use egui_file_dialog::FileDialog;

// If the viewer is shown or not marker component
#[derive(Component)]
pub struct ShowDataViewer;

/// Which column the class list is currently sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DvSortColumn {
    /// Preserve the natural class-index order, default.
    #[default]
    None,
    /// Sort by the Unicode character glyph code point.
    Char,
    /// Sort alphabetically by the Unicode character name.
    Name,
    /// Sort by sample count.
    Count,
}

/// Sort direction for the class list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DvSortDir {
    #[default]
    Asc,
    Desc,
}

/// Cache of processed pixel buffers, here for letting me swap between raw
/// images and processed.
///
/// This isn't intended to be photoshop or anything just a quick and very crappy
/// way to toggle raw test images and apply different cleanups. I had planned on
/// trying to get cute with all this but it probably won't help much for the CNN
/// training anyway.
///
/// Automatically invalided on any change to the effects. Causes the entire
/// ecs+bevy+ui to block cause I'm a hack that didn't bother making this
/// interactive.
#[derive(Resource, Default)]
pub struct ImageProcessingCache {
    /// Processed pixel buffers. Key is the flat image index inside the loaded
    /// dataset; value is the final pipeline output data.
    pub entries: std::collections::HashMap<usize, Vec<u8>>,
    /// Per-image pipeline quality score in `[0.0, 1.0]`. Only populated when
    /// a threshold mode is active, None = no entry. Cleared together with
    /// `entries` whenever the pipeline key changes or a new dataset is loaded.
    /// N > 0.6 = good, 0.3–0.6 = sus, < 0.3 = ass. Not sure this is worth keeping.
    pub quality: std::collections::HashMap<usize, f32>,
    /// Which method `ThresholdMode::Auto` chose for each image. Only
    /// populated when Auto is selected.
    pub auto_method: std::collections::HashMap<usize, AutoMethod>,
    /// Serialiszd snapshot of all pipeline parameters at the time the cache
    /// was last populated. Compared each frame, if it changes causes recompute
    /// of the image pipeline for the entire class.
    pub pipeline_key: String,
}

/// All backing ui and data state for the Data Viewer window.
#[derive(Resource)]
pub struct DataViewerState {
    /// Directory path text field.
    pub dir: String,
    /// Successfully loaded dataset, or `None` if nothing loaded yet.
    pub dataset: Option<LoadedDataset>,
    /// Currently selected class index.
    pub selected_class: usize,
    /// Text filter applied to the class list cause 7k classes makes it hard to find crap.
    pub filter: String,
    /// Status or error string shown below the Load button.
    pub status: String,
    /// Pixel scale how many screen pixels each image occupies.
    pub px_scale: f32,
    /// Active sort column for the class list.
    pub sort_col: DvSortColumn,
    /// Active sort direction for the class list.
    pub sort_dir: DvSortDir,
    /// Show all images instead of just a limited set (120 max).
    pub show_all_images: bool,
    /// When true, display the original image instead of the processed image.
    /// The processed image cache is still populated so toggling back to the
    /// processed view is instant with no recomputation.
    pub show_original: bool,
    // The filters are applied in this order, I had throught about making it
    // dynamic but after testing out trainig with worse approaches not sure it
    // matters. Will probably rip this entire chain out.
    // Prefilters: median, gaussian, bg_normalize, contrast_stretch
    //
    // Thresholds: None | Otsu | Sauvola | Auto
    //
    // Postfilters:  morph_open, morph_close, min_component
    /// Apply a median filter before thresholding, removes isolated noise of a
    /// certain size to help otsu out. Seems minimally useful.
    pub median_prefilter: bool,
    /// Median filter radius: 1 = 3x3, 2 = 5x5.
    pub median_radius: usize,
    /// Apply a Gaussian blur before thresholding reduce noise.
    pub gaussian_prefilter: bool,
    /// Gaussian blur σ in pixels.
    pub gaussian_sigma: f32,
    /// Estimate and subtract the slowly-varying background illumination using a
    /// large Gaussian. Normalize "paper on desk" images so Otsu sees a clean
    /// two-cluster problem. A lot of the ETL dataset has this issue.
    pub bg_normalize: bool,
    /// Sigma as a fraction of max(width, height). Default 0.15 -> ~10 px for 64 px images.
    pub bg_sigma_scale: f32,
    /// Stretch the histogram to the full 0-255 range before thresholding.
    pub contrast_stretch_pre: bool,
    /// Percentage of pixels to clip from each end of the histogram.
    pub contrast_clip_pct: f32,

    /// Which thresholding algorithm to apply, None = show original grey.
    pub threshold_mode: ThresholdMode,
    /// Invert the threshold output.
    pub threshold_invert: bool,
    /// Sauvola local window half-size in pixels.
    pub sauvola_window: usize,
    /// Sauvola k factor. Controls how aggressively local contrast drives the
    /// threshold aka higher values threshold more of the image as foreground.
    // I barely get all the knobs with this algorithm its output is so weird
    // like a donut around things that matter. How the CNN tolerates this jank
    // is hilarious.
    pub sauvola_k: f32,
    /// Morphological open erode to dilate on the output, removes small isolated
    /// noise blobs without altering stroke shapes that should survive
    pub morph_open: bool,
    /// Morphological close dilate to erode on the output fills small holes and
    /// gaps inside strokes. Basically useless for kanji I found.
    // TODO: REMOVEME
    pub morph_close: bool,
    /// Structuring-element radius shared by open and close fn's (1 = 3x3, 2 = 5x5, 3 = 7x7).
    pub morph_radius: usize,
    /// Remove connected foreground components smaller than `min_component_size` pixels.
    pub min_component: bool,
    /// Minimum blob area in pixels; smaller blobs are zeroed out.
    pub min_component_size: usize,

    /// In-process egui folder-picker, only on native builds
    #[cfg(not(target_arch = "wasm32"))]
    pub file_dialog: FileDialog,
}

impl Default for DataViewerState {
    fn default() -> Self {
        // Default to the process current working directory on native builds to make things easier.
        let cwd = if let Ok(p) = std::env::current_dir() {
            p.to_string_lossy().into_owned()
        } else {
            String::new()
        };

        Self {
            dir: cwd,
            dataset: None,
            selected_class: 0,
            filter: String::new(),
            status: "Enter a dataset directory and then click Load.".into(),
            px_scale: 3.0,
            sort_col: DvSortColumn::default(),
            sort_dir: DvSortDir::default(),
            show_all_images: false,
            show_original: false,
            median_prefilter: false,
            median_radius: 1,
            gaussian_prefilter: false,
            gaussian_sigma: 1.0,
            bg_normalize: false,
            bg_sigma_scale: 0.15,
            contrast_stretch_pre: false,
            contrast_clip_pct: 1.0,
            threshold_mode: ThresholdMode::default(),
            threshold_invert: false,
            sauvola_window: 11,
            sauvola_k: 0.2,
            morph_open: false,
            morph_close: false,
            morph_radius: 1,
            min_component: false,
            min_component_size: 10,
            #[cfg(not(target_arch = "wasm32"))]
            file_dialog: FileDialog::new(),
        }
    }
}

/// An in-memory npz dataset to use for display purposes.
pub struct LoadedDataset {
    /// Flat pixel buffer for all images `n_images x channels x img_height x
    /// img_width` bytes to save on pointer chasing.
    ///
    /// For 3-channel npz files channel 0 is the original grayscale image,
    /// channel 1 is Otsu, channel 2 is Sauvola. The viewer only
    /// displays/processes channel 0.
    images: Vec<u8>,
    /// TODO: REMOVEME?
    /// One label per image, u32 to fit the 2-7k kanji classes.
    #[allow(dead_code)]
    labels: Vec<u32>,
    /// Class index to Unicode char.
    pub classmap: Vec<char>,
    /// Class index to sorted list of image indices with that label.
    pub by_class: Vec<Vec<usize>>,
    pub n_images: usize,
    pub n_classes: usize,
    /// Native pixel width of each image as stored in the npz.
    pub img_width: usize,
    /// Native pixel height of each image as stored in the npz.
    pub img_height: usize,
    /// Channels per image as stored in the npz: 1 for plain (N,H,W) arrays,
    /// 3 for (N,C,H,W) three-channel (original, Otsu, Sauvola) arrays.
    pub channels: usize,
}

impl LoadedDataset {
    /// Borrow the pixel slice for image `idx`, length is `img_height *
    /// img_width`.
    ///
    /// Only channel 0 (the original grayscale image) is returned even when
    /// the source npz is multi-channel - the rest of the pipeline here is
    /// single-channel grayscale only.
    pub fn image(&self, idx: usize) -> &[u8] {
        let ppi = self.img_height * self.img_width;
        let stride = self.channels * ppi;
        let start = idx * stride;
        &self.images[start..start + ppi]
    }

    /// Character for class `idx`, or `'?'` if out of range.
    pub fn class_char(&self, idx: usize) -> char {
        self.classmap.get(idx).copied().unwrap_or('?')
    }

    /// Number of samples in class `idx`.
    pub fn class_count(&self, idx: usize) -> usize {
        self.by_class.get(idx).map(|v| v.len()).unwrap_or(0)
    }
}

/// Try to load an npz dataset from `dir`. Returns whatever error string is sent
/// back on failure.
///
/// Looks for npz files in this order:
///  - `train-imgs.npz` && `train-labels.npz` (split output for training)
///  - `imgs.npz` && `labels.npz` (chungus unsplit output)
///
/// The first pair found is used if both exist cause I forgot to remove files in
/// a dest directory. The `classmap.json` file needs to exist in all cases.
///
/// Not supported on WASM as file I/O requires the native target, future me can
/// brain up how to deal with files in http dynamically. For now whatever.
pub fn try_load(dir: &str) -> Result<LoadedDataset, String> {
    #[cfg(target_arch = "wasm32")]
    return Err("file loading is not supported on the web build".into());

    #[cfg(not(target_arch = "wasm32"))]
    {
        // Probe for the first pair where both files exist first.
        let candidates = [
            ("train-imgs.npz", "train-labels.npz"),
            ("imgs.npz", "labels.npz"),
        ];
        let (imgs_name, labels_name) = candidates
            .iter()
            .find(|(i, l)| {
                std::path::Path::new(&format!("{dir}/{i}")).exists()
                    && std::path::Path::new(&format!("{dir}/{l}")).exists()
            })
            .copied()
            .ok_or_else(|| {
                format!(
                    "{dir}: no npz pair found to load, expected train-imgs.npz+train-labels.npz or imgs.npz+labels.npz this is a you problem"
                )
            })?;

        let imgs_path = format!("{dir}/{imgs_name}");
        let labels_path = format!("{dir}/{labels_name}");
        let classmap_path = format!("{dir}/classmap.json");

        let imgs_raw = lib::npz::read_npz_first_entry(&imgs_path)?;
        let imgs_hdr = lib::npz::parse_npy_header(&imgs_raw, &imgs_path)?;
        if imgs_hdr.dtype != "|u1" {
            return Err(format!(
                "{imgs_path}: expected |u1 images, got {}",
                imgs_hdr.dtype
            ));
        }
        // Accept plain (N, H, W) grayscale arrays as well as (N, C, H, W)
        // arrays from `ma convert --three-channel` (original/Otsu/Sauvola
        // stacked as channels). Anything else is a you problem.
        if imgs_hdr.shape.len() != 3 && imgs_hdr.shape.len() != 4 {
            return Err(format!(
                "{imgs_path}: expected a 3-D (N,H,W) or 4-D (N,C,H,W) array, got shape {:?} instead?",
                imgs_hdr.shape
            ));
        }
        let n_images = imgs_hdr.shape[0];
        let (channels, img_height, img_width) = match imgs_hdr.shape.len() {
            3 => (1, imgs_hdr.shape[1], imgs_hdr.shape[2]),
            4 => (imgs_hdr.shape[1], imgs_hdr.shape[2], imgs_hdr.shape[3]),
            _ => unreachable!("checked above"),
        };
        let img_data = imgs_raw[imgs_hdr.data_offset..].to_vec();

        let lbl_raw = lib::npz::read_npz_first_entry(&labels_path)?;
        let lbl_hdr = lib::npz::parse_npy_header(&lbl_raw, &labels_path)?;
        if lbl_hdr.shape.len() != 1 || lbl_hdr.shape[0] != n_images {
            return Err(format!(
                "{labels_path}: expected {n_images}, got {:?}",
                lbl_hdr.shape
            ));
        }
        let lbl_data = &lbl_raw[lbl_hdr.data_offset..];
        let labels = lib::npz::read_labels(lbl_data, n_images, &lbl_hdr.dtype);

        let classmap_text = std::fs::read_to_string(&classmap_path)
            .map_err(|e| format!("cannot read {classmap_path}: {e}"))?;
        let raw_strs: Vec<String> = serde_json::from_str(&classmap_text)
            .map_err(|e| format!("cannot parse {classmap_path}: {e}"))?;
        let classmap: Vec<char> = raw_strs.iter().filter_map(|s| s.chars().next()).collect();
        let n_classes = classmap.len();

        let mut by_class: Vec<Vec<usize>> = vec![Vec::new(); n_classes];
        for (img_idx, &label) in labels.iter().enumerate() {
            let cls = label as usize;
            if cls < n_classes {
                by_class[cls].push(img_idx);
            }
        }

        Ok(LoadedDataset {
            images: img_data,
            labels,
            classmap,
            by_class,
            n_images,
            n_classes,
            img_width,
            img_height,
            channels,
        })
    }
}

/// Advance the sort state when a column header is clicked.
///
/// State is trinary, off/default or asc/desc. Click order is
/// ascending/descending/default.
fn dv_cycle_sort(cur: DvSortColumn, clicked: DvSortColumn, dir: &mut DvSortDir) -> DvSortColumn {
    if cur == clicked {
        if *dir == DvSortDir::Asc {
            *dir = DvSortDir::Desc;
            clicked
        } else {
            *dir = DvSortDir::Asc;
            DvSortColumn::None
        }
    } else {
        *dir = DvSortDir::Asc;
        clicked
    }
}

/// Render a clickable column-header button with a sort-direction indicator.
///
/// Shows `▲` / `▼` when this column is the active sort, `⇅` otherwise. The
/// button text is highlighted when actively sorting. Returns `true` if clicked.
// TODO: Is there an egui lib for this crap?
fn dv_header_btn(
    ui: &mut egui::Ui,
    label: &str,
    col: DvSortColumn,
    cur_col: DvSortColumn,
    cur_dir: DvSortDir,
) -> bool {
    let indicator = if cur_col == col {
        if cur_dir == DvSortDir::Asc {
            " ▲"
        } else {
            " ▼"
        }
    } else {
        " ⇅"
    };
    let text = egui::RichText::new(format!("{label}{indicator}"))
        .strong()
        .color(if cur_col == col {
            egui::Color32::from_rgb(220, 220, 255)
        } else {
            egui::Color32::from_rgb(150, 150, 210)
        });
    ui.button(text).clicked()
}

/// Return the Unicode standard name for `ch` in title case, aka convert the:
/// ANGRY_INTERNET_TROLL unicode name to Angry Internet Troll. Empty string if
/// no unicode name comes back
fn char_unicode_name(ch: char) -> String {
    unicode_names2::name(ch)
        .map(|n| {
            let s = n.to_string();
            let mut title = String::with_capacity(s.len());
            let mut next_upper = true;
            for c in s.chars() {
                if c == ' ' || c == '-' {
                    title.push(c);
                    next_upper = true;
                } else if next_upper {
                    title.extend(c.to_uppercase());
                    next_upper = false;
                } else {
                    title.extend(c.to_lowercase());
                }
            }
            title
        })
        .unwrap_or_default()
}

/// Draw a single grayscale image as colored rects.
///
/// Accepts any `width x height`; the image is scaled up by `scale` screen
/// pixels per image pixel  Near-zero pixels are skipped for performance and
/// left as white.
fn draw_image(
    ui: &mut egui::Ui,
    pixels: &[u8],
    width: usize,
    height: usize,
    scale: f32,
) -> egui::Response {
    let size = egui::vec2(width as f32 * scale, height as f32 * scale);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, egui::Color32::WHITE);

        for row in 0..height {
            for col in 0..width {
                let v = pixels[row * width + col];
                if v < 8 {
                    continue;
                }
                let gray = 255u8.saturating_sub(v);
                let color = egui::Color32::from_gray(gray);
                let px = egui::Rect::from_min_size(
                    rect.min + egui::vec2(col as f32 * scale, row as f32 * scale),
                    egui::vec2(scale, scale),
                );
                painter.rect_filled(px, 0.0, color);
            }
        }
    }

    response
}

/// Build a short string that uniquely identifies the current pipeline
/// configuration.
///
/// This is just an oracle for detecting in the image cache pipeline that we
/// need to recompute everything if any ui inputs changed. Nothing special.
fn make_pipeline_key(state: &DataViewerState) -> String {
    format!(
        "{},{},{},{:08x},{},{:08x},{},{:08x},{},{},{},{:08x},{},{},{},{},{}",
        state.median_prefilter as u8,
        state.median_radius,
        state.gaussian_prefilter as u8,
        state.gaussian_sigma.to_bits(),
        state.bg_normalize as u8,
        state.bg_sigma_scale.to_bits(),
        state.contrast_stretch_pre as u8,
        state.contrast_clip_pct.to_bits(),
        state.threshold_mode as u8,
        state.threshold_invert as u8,
        state.sauvola_window,
        state.sauvola_k.to_bits(),
        state.morph_open as u8,
        state.morph_close as u8,
        state.morph_radius,
        state.min_component as u8,
        state.min_component_size,
    )
}

/// Bevy system that renders the Data Viewer egui window.
pub fn data_viewer_window(
    mut contexts: EguiContexts,
    query: Query<Entity, With<ShowDataViewer>>,
    mut state: ResMut<DataViewerState>,
    mut cache: ResMut<ImageProcessingCache>,
    mut commands: Commands,
) -> Result {
    if query.is_empty() {
        return Ok(());
    }

    // Pipeline options changed, invalidate the cache so its recomputed.
    let current_key = make_pipeline_key(&state);
    if current_key != cache.pipeline_key {
        cache.entries.clear();
        cache.quality.clear();
        cache.auto_method.clear();
        cache.pipeline_key = current_key;
    }

    let mut open = true;

    egui::Window::new("Data Viewer")
        .open(&mut open)
        .default_size([1920.0, 1080.0])
        .resizable(true)
        .show(contexts.ctx_mut()?, |ui: &mut egui::Ui| {
            ui.horizontal(|ui| {
                ui.label("Directory:");
                ui.add(
                    egui::TextEdit::singleline(&mut state.dir)
                        .hint_text("dir with npz training files within")
                        .desired_width(300.0),
                );
                if ui.button("Load").clicked() {
                    let dir = state.dir.trim().to_owned();
                    match try_load(&dir) {
                        Ok(ds) => {
                            state.status = format!(
                                "Loaded {} images, {} classes, {}x{} px from {dir}",
                                ds.n_images, ds.n_classes, ds.img_width, ds.img_height,
                            );
                            state.selected_class = 0;
                            state.dataset = Some(ds);
                            cache.entries.clear();
                            cache.quality.clear();
                            cache.auto_method.clear();
                        }
                        Err(e) => {
                            state.status = format!("Error: {e}");
                            state.dataset = None;
                        }
                    }
                }

                // Browse button, native only opens the egui filesystem picker widget.
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Browse...").clicked() {
                    use std::path::PathBuf;
                    let start = PathBuf::from(&state.dir);
                    state.file_dialog = FileDialog::new().initial_directory(start);
                    state.file_dialog.pick_directory();
                }

                ui.add_space(8.0);
                ui.checkbox(&mut state.show_all_images, "Show all images");
                ui.add_space(4.0);

                // Toggle between processed pipeline output and the original raw pixels.
                // The cache is always kept warm so flipping is instant if
                // everythings been computed.
                ui.checkbox(&mut state.show_original, "Show original")
                    .on_hover_text(
                        "Display the raw unprocessed image instead of the pipeline output.\n\
                         The processed cache is still maintained so toggling back is instant.",
                    );

                // TODO: I'm not sure this is useful to keep its janky and I'm sick of the resize logic
                ui.label("Zoom:");
                ui.add(egui::Slider::new(&mut state.px_scale, 1.0..=6.0).step_by(0.5));
            });

            ui.label(egui::RichText::new(&state.status).weak().italics().small());
            ui.separator();

            ui.collapsing("⚙ Image Processing", |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Pre-filter:").strong());
                    ui.add_space(4.0);

                    ui.checkbox(&mut state.median_prefilter, "Median");
                    if state.median_prefilter {
                        egui::ComboBox::from_id_salt("median_kernel")
                            .selected_text(if state.median_radius == 1 {
                                "3x3"
                            } else {
                                "5x5"
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut state.median_radius, 1, "3x3");
                                ui.selectable_value(&mut state.median_radius, 2, "5x5");
                            });
                        ui.add_space(4.0);
                    }

                    ui.checkbox(&mut state.gaussian_prefilter, "Gaussian");
                    if state.gaussian_prefilter {
                        ui.label("σ:");
                        ui.add(
                            egui::Slider::new(&mut state.gaussian_sigma, 0.5..=3.0)
                                .step_by(0.25)
                                .max_decimals(2),
                        );
                        ui.add_space(4.0);
                    }

                    ui.checkbox(&mut state.bg_normalize, "BG Normalize");
                    if state.bg_normalize {
                        ui.label("σ scale:");
                        ui.add(
                            egui::Slider::new(&mut state.bg_sigma_scale, 0.05..=0.40)
                                .step_by(0.05)
                                .max_decimals(2),
                        );
                        ui.add_space(4.0);
                    }

                    ui.checkbox(&mut state.contrast_stretch_pre, "Contrast Stretch");
                    if state.contrast_stretch_pre {
                        ui.label("clip %:");
                        ui.add(
                            egui::Slider::new(&mut state.contrast_clip_pct, 0.0..=5.0)
                                .step_by(0.25)
                                .max_decimals(2),
                        );
                    }
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Threshold:").strong());
                    ui.add_space(4.0);

                    ui.radio_value(&mut state.threshold_mode, ThresholdMode::None, "None");
                    ui.radio_value(&mut state.threshold_mode, ThresholdMode::Otsu, "Otsu");
                    ui.radio_value(
                        &mut state.threshold_mode,
                        ThresholdMode::Sauvola,
                        "Sauvola",
                    );
		    // TODO: Nuke Auto mode won't need it it made trains WORSE
		    // and for the CNN to just detect the threshold instead.
		    // Which should have been obvious.
                    ui.radio_value(&mut state.threshold_mode, ThresholdMode::Auto, "Auto");

                    if state.threshold_mode != ThresholdMode::None {
                        ui.add_space(4.0);
                        ui.checkbox(&mut state.threshold_invert, "Invert");
                    }

                    if state.threshold_mode == ThresholdMode::Sauvola {
                        ui.add_space(8.0);
                        ui.label("Window:");
                        ui.add(
                            egui::Slider::new(&mut state.sauvola_window, 3..=31)
                                .step_by(2.0)
                                .suffix("px"),
                        );
                        ui.add_space(4.0);
                        ui.label("k:");
                        ui.add(
                            egui::Slider::new(&mut state.sauvola_k, 0.05..=0.5)
                                .step_by(0.05)
                                .max_decimals(2),
                        );
                    }
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Post-filter:").strong());
                    ui.add_space(4.0);

                    ui.checkbox(&mut state.morph_open, "Open");
                    ui.checkbox(&mut state.morph_close, "Close");
                    if state.morph_open || state.morph_close {
                        ui.label("Radius:");
                        ui.add(egui::Slider::new(&mut state.morph_radius, 1..=3));
                        ui.add_space(4.0);
                    }

                    ui.checkbox(&mut state.min_component, "Min blob:");
                    if state.min_component {
                        ui.add(
                            egui::Slider::new(&mut state.min_component_size, 2..=50)
                                .suffix("px²"),
                        );
                    }

                    let post_active =
                        state.morph_open || state.morph_close || state.min_component;
                    if post_active && state.threshold_mode == ThresholdMode::None {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("post-filters need a threshold active dumdum")
                                .weak()
                                .small(),
                        );
                    }
                });
            });
            ui.separator();

            if state.dataset.is_none() {
                ui.label("No dataset loaded.");
                return;
            }

            const LEFT_W: f32 = 200.0;
            const GRID_GAP: f32 = 4.0;

            let selected = state.selected_class;
            let scale = state.px_scale;
            let filter_text = state.filter.clone();
            let filter_lc = filter_text.to_lowercase();
            let threshold_mode = state.threshold_mode;
            let threshold_invert = state.threshold_invert;
            let median_prefilter = state.median_prefilter;
            let median_radius = state.median_radius;
            let gaussian_prefilter = state.gaussian_prefilter;
            let gaussian_sigma = state.gaussian_sigma;
            let bg_normalize_flag = state.bg_normalize;
            let bg_sigma_scale = state.bg_sigma_scale;
            let contrast_stretch_flag = state.contrast_stretch_pre;
            let contrast_clip_pct = state.contrast_clip_pct;
            let sauvola_window = state.sauvola_window;
            let sauvola_k = state.sauvola_k;
            let morph_open_flag = state.morph_open;
            let morph_close_flag = state.morph_close;
            let morph_radius = state.morph_radius;
            let min_component_flag = state.min_component;
            let min_component_size = state.min_component_size;
            let show_original = state.show_original;

            ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                ui.vertical(|ui| {
                    ui.set_width(LEFT_W);
                    ui.label(egui::RichText::new("Classes").strong());
                    ui.add(
                        egui::TextEdit::singleline(&mut state.filter)
                            .hint_text("filter...")
                            .desired_width(LEFT_W - 8.0),
                    )
			.changed();

                    // dataset.is_none() early return is above this block.
                    let ds = state.dataset.as_ref().expect("dataset is Some; guarded above");

                    // Build the list of class indices, then sort.
                    let mut matching: Vec<usize> = (0..ds.n_classes)
                        .filter(|&idx| {
                            if filter_lc.is_empty() {
                                return true;
                            }
                            let ch = ds.class_char(idx);
                            ch.to_string().contains(&filter_text)
                                || idx.to_string().contains(&filter_lc)
                                || char_unicode_name(ch).to_lowercase().contains(&filter_lc)
                        })
                        .collect();

                    let sort_col = state.sort_col;
                    let sort_dir = state.sort_dir;
                    if sort_col != DvSortColumn::None {
                        // Pre-compute the sort key for each class so
                        // char_unicode_name() is called once per class, not
                        // twice per comparison not sure if needed anymore.
                        let keys: Vec<(char, String, usize)> = (0..ds.n_classes)
                            .map(|idx| {
                                let ch = ds.class_char(idx);
                                (ch, char_unicode_name(ch), ds.class_count(idx))
                            })
                            .collect();
                        matching.sort_by(|&a, &b| {
                            let ord = match sort_col {
                                DvSortColumn::None => std::cmp::Ordering::Equal,
                                DvSortColumn::Char => keys[a].0.cmp(&keys[b].0),
                                DvSortColumn::Name => keys[a].1.cmp(&keys[b].1),
                                DvSortColumn::Count => keys[a].2.cmp(&keys[b].2),
                            };
                            if sort_dir == DvSortDir::Desc {
                                ord.reverse()
                            } else {
                                ord
                            }
                        });
                    }

                    // Column header buttons for sorting. Same three-state cycle
                    // used by the World Clock and I should make this a lib
                    // thing soon in a refactor.
                    let mut new_sort: Option<(DvSortColumn, DvSortDir)> = None;
                    ui.horizontal(|ui| {
                        let mut tmp_dir = sort_dir;
                        if dv_header_btn(ui, "Char", DvSortColumn::Char, sort_col, sort_dir) {
                            let col = dv_cycle_sort(sort_col, DvSortColumn::Char, &mut tmp_dir);
                            new_sort = Some((col, tmp_dir));
                        } else if dv_header_btn(ui, "Name", DvSortColumn::Name, sort_col, sort_dir)
                        {
                            let col = dv_cycle_sort(sort_col, DvSortColumn::Name, &mut tmp_dir);
                            new_sort = Some((col, tmp_dir));
                        } else if dv_header_btn(ui, "N", DvSortColumn::Count, sort_col, sort_dir) {
                            let col = dv_cycle_sort(sort_col, DvSortColumn::Count, &mut tmp_dir);
                            new_sort = Some((col, tmp_dir));
                        }
                    });

                    let mut new_sel = selected;
                    // Calculate dynamic height based on available space in the window
                    let available_height = ui.available_height();
                    // Reserve space for headers, filter input, column buttons, and padding
                    let header_height = 140.0; // rough swag for all gooey elements above scroll area
                    let max_height = (available_height - header_height).max(200.0);

                    egui::ScrollArea::vertical()
                        .id_salt("class_list")
                        .max_height(max_height)
                        .show(ui, |ui| {
                            for &idx in &matching {
                                let ch = ds.class_char(idx);
                                let count = ds.class_count(idx);
                                let name = char_unicode_name(ch);
                                let label = if name.is_empty() {
                                    format!("{ch}  ({count})")
                                } else {
                                    format!("{ch} {name} ({count})")
                                };
                                if ui.selectable_label(idx == selected, &label).clicked() {
                                    new_sel = idx;
                                }
                            }
                        });
                    state.selected_class = new_sel;

                    if let Some((col, dir)) = new_sort {
                        state.sort_col = col;
                        state.sort_dir = dir;
                    }
                });

                ui.separator();

                ui.vertical(|ui| {
                    // dataset.is_none() early return is above this block.
                    let ds = state.dataset.as_ref().expect("dataset is Some; guarded above");
                    let sel = state.selected_class;
                    let ch = ds.class_char(sel);
                    let count = ds.class_count(sel);
                    let iw = ds.img_width;
                    let ih = ds.img_height;
                    let img_w_px = iw as f32 * scale;

                    let available_width = ui.available_width();
                    let right_w = available_width - LEFT_W - 12.0;
                    let per_row = ((right_w + GRID_GAP) / (img_w_px + GRID_GAP)).floor() as usize;
                    let per_row = per_row.max(1);

                    let indices = ds.by_class.get(sel).map(|v| v.as_slice()).unwrap_or(&[]);
                    let show_n = if state.show_all_images {
                        indices.len()
                    } else {
                        indices.len().min(120)
                    };

                    let ch_name = char_unicode_name(ch);
                    let header = if ch_name.is_empty() {
                        format!("Class {sel} = '{ch}'  ({count} samples)  [{iw}x{ih}]")
                    } else {
                        format!("Class {sel} = '{ch}' {ch_name}  ({count} samples)  [{iw}x{ih}]")
                    };

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&header).strong());

                        // Quality aggregate, only shown when a threshold is
                        // active. Reads from the previous frame's cache so its
                        // not "real time" but no human will figure this out
                        if threshold_mode != ThresholdMode::None {
                            let scores: Vec<f32> = indices[..show_n]
                                .iter()
                                .filter_map(|idx| cache.quality.get(idx))
                                .copied()
                                .collect();

                            if !scores.is_empty() {
                                let n = scores.len() as f32;
                                let mean = scores.iter().sum::<f32>() / n;
                                let good =
                                    scores.iter().filter(|&&s| s >= 0.6).count();
                                let suspect =
                                    scores.iter().filter(|&&s| (0.3..0.6).contains(&s)).count();
                                let bad =
                                    scores.iter().filter(|&&s| s < 0.3).count();

                                let color = if mean >= 0.6 {
                                    egui::Color32::from_rgb(100, 200, 100)
                                } else if mean >= 0.3 {
                                    egui::Color32::from_rgb(220, 180, 50)
                                } else {
                                    egui::Color32::from_rgb(220, 80, 80)
                                };

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "Q {mean:.2}  ✓{good} ⚠{suspect} ✗{bad}"
                                            ))
						.color(color)
						.monospace()
						.small(),
                                        )
                                            .on_hover_text(
						"Pipeline quality for visible images.\n\
						 Q = 0.6xFDR_tent + 0.4x(otsu_var/5000)\n\
						 W >= 0.60  ⚠ 0.30–0.59  L < 0.30\n\
						 Near-blank or near-solid outputs score 0 or near to it.",
                                            );
                                    },
                                );
                            }
                        }
                    });

                    ui.add_space(4.0);

                    egui::ScrollArea::vertical()
                        .id_salt("img_grid")
                        .show(ui, |ui| {
                            for chunk in indices[..show_n].chunks(per_row) {
                                ui.horizontal(|ui| {
                                    for &img_idx in chunk {
					// Do we draw from an existing cache or not?
                                        if let std::collections::hash_map::Entry::Vacant(e) = cache.entries.entry(img_idx) {
                                            let raw = ds.image(img_idx);
                                            let cfg = ImagePipelineConfig {
                                                median_prefilter,
                                                median_radius,
                                                gaussian_prefilter,
                                                gaussian_sigma,
                                                bg_normalize: bg_normalize_flag,
                                                bg_sigma_scale,
                                                contrast_stretch: contrast_stretch_flag,
                                                contrast_clip_pct,
                                                threshold_mode,
                                                threshold_invert,
                                                sauvola_window,
                                                sauvola_k,
                                                morph_open: morph_open_flag,
                                                morph_close: morph_close_flag,
                                                morph_radius,
                                                min_component: min_component_flag,
                                                min_component_size,
                                            };
                                            let result =
                                                run_pipeline(raw, iw, ih, &cfg);
                                            e.insert(result.pixels);
                                            if let Some(q) = result.quality {
                                                cache.quality.insert(img_idx, q);
                                            }
                                            if let Some(m) = result.auto_method {
                                                cache.auto_method.insert(img_idx, m);
                                            }
                                        }

					// TODO: Removeme?
                                        // Wrap in a vertical container so the label sits
                                        // directly above its image without hovering.
                                        ui.vertical(|ui| {
                                            let q_opt = cache.quality.get(&img_idx).copied();

                                            // FDR from cached pixels.
                                            let fdr_opt: Option<f32> =
                                                if threshold_mode != ThresholdMode::None {
                                                    cache.entries.get(&img_idx).map(|px| {
                                                        let fg = px
                                                            .iter()
                                                            .filter(|&&b| b == 255)
                                                            .count()
                                                            as f32;
                                                        fg / px.len() as f32
                                                    })
                                                } else {
                                                    None
                                                };

                                            let (label_text, label_color) =
                                                if threshold_mode == ThresholdMode::Auto {
                                                    let method_short =
                                                        match cache.auto_method.get(&img_idx) {
                                                            Some(AutoMethod::Otsu) => "Otsu",
                                                            Some(AutoMethod::SauvolaWide) => "S11",
                                                            Some(AutoMethod::SauvolaNarrow) => "S7",
                                                            None => "...",
                                                        };
                                                    let q = q_opt.unwrap_or(0.0);
                                                    let color = if q >= 0.6 {
                                                        egui::Color32::from_rgb(100, 200, 100)
                                                    } else if q >= 0.3 {
                                                        egui::Color32::from_rgb(220, 180, 50)
                                                    } else {
                                                        egui::Color32::from_rgb(220, 80, 80)
                                                    };
                                                    let fdr_str = fdr_opt
                                                        .map(|f| format!(" {:.0}%", f * 100.0))
                                                        .unwrap_or_default();
                                                    (
                                                        format!("{method_short} {q:.2}{fdr_str}"),
                                                        color,
                                                    )
                                                } else if threshold_mode != ThresholdMode::None {
                                                    let q = q_opt.unwrap_or(0.0);
                                                    let color = if q >= 0.6 {
                                                        egui::Color32::from_rgb(100, 200, 100)
                                                    } else if q >= 0.3 {
                                                        egui::Color32::from_rgb(220, 180, 50)
                                                    } else {
                                                        egui::Color32::from_rgb(220, 80, 80)
                                                    };
                                                    let fdr_str = fdr_opt
                                                        .map(|f| format!(" {:.0}%", f * 100.0))
                                                        .unwrap_or_default();
                                                    (format!("{q:.2}{fdr_str}"), color)
                                                } else {
                                                    (
                                                        format!("#{img_idx}"),
                                                        egui::Color32::from_gray(130),
                                                    )
                                                };

                                            ui.label(
                                                egui::RichText::new(label_text)
                                                    .monospace()
                                                    .small()
                                                    .color(label_color),
                                            );

                                            let resp = if show_original {
                                                draw_image(
                                                    ui,
                                                    ds.image(img_idx),
                                                    iw,
                                                    ih,
                                                    scale,
                                                )
                                            } else {
                                                let px = cache
                                                    .entries
                                                    .get(&img_idx)
                                                    .expect("img_idx must be in cache when show_original is false");
                                                draw_image(ui, px, iw, ih, scale)
                                            };

                                            // Full tooltip: method, Q score, and FDR for debugging.
                                            let fdr_line = fdr_opt
                                                .map(|f| {
                                                    format!("\nFDR {:.1}%", f * 100.0)
                                                })
                                                .unwrap_or_default();
                                            let hover =
                                                if threshold_mode == ThresholdMode::Auto {
                                                    let method_str = cache
                                                        .auto_method
                                                        .get(&img_idx)
                                                        .map(|m| m.to_string())
                                                        .unwrap_or_else(|| "...".into());
                                                    let q_str = q_opt
                                                        .map(|q| format!("{q:.2}"))
                                                        .unwrap_or_else(|| "?".into());
                                                    format!(
                                                        "sample {img_idx}\nAuto = {method_str}\nQ {q_str}{fdr_line}"
                                                    )
                                                } else {
                                                    format!("sample {img_idx}{fdr_line}")
                                                };
                                            resp.on_hover_text(hover);
                                        });
                                        ui.add_space(GRID_GAP);
                                    }
                                });
                                ui.add_space(GRID_GAP);
                            }

                            if show_n < indices.len() {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "... {} more samples not shown by default",
                                        indices.len() - show_n
                                    ))
					.weak()
					.italics(),
                                );
                            } else if !state.show_all_images {
                                ui.label(
                                    egui::RichText::new("Showing only 120 samples")
                                        .weak()
                                        .italics(),
                                );
                            }
                        });
                });
            });
        });

    if !open && let Ok(entity) = query.single() {
        commands.entity(entity).despawn();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let ctx = contexts.ctx_mut()?;
        state.file_dialog.update(ctx);
        if let Some(path) = state.file_dialog.take_picked() {
            state.dir = path.to_string_lossy().into_owned();
        }
    }

    Ok(())
}
