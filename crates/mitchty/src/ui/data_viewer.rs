//! Data viewer window to inspect npz training datasets visually cause some of
//! these datasets are... not obviously bad until you look at them.

use std::io::Read;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

// Native-only egui file dialog for folder selection; not compiled for WASM cause no fs
#[cfg(not(target_arch = "wasm32"))]
use egui_file_dialog::FileDialog;

// If the viewer is shown or not marker component
#[derive(Component)]
pub struct ShowDataViewer;

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
    /// Horizontal scroll position of the image grid (for future maybe I forgot why I added this)
    #[allow(dead_code)]
    pub grid_scroll: f32,
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
            grid_scroll: 0.0,
            #[cfg(not(target_arch = "wasm32"))]
            file_dialog: FileDialog::new(),
        }
    }
}

/// An in-memory npz dataset to use for display purposes.
pub struct LoadedDataset {
    /// Flat pixel buffer for all the images.
    images: Vec<u8>,
    /// One label per image, u32 to fit the 7k kanji classes
    #[allow(dead_code)]
    labels: Vec<u32>,
    /// Class index to Unicode char.
    pub classmap: Vec<char>,
    /// Class index to sorted list of image indices with that label.
    pub by_class: Vec<Vec<usize>>,
    pub n_images: usize,
    pub n_classes: usize,
}

impl LoadedDataset {
    /// Borrow the 784-byte pixel slice for the image `idx` index.
    pub fn image(&self, idx: usize) -> &[u8] {
        &self.images[idx * 784..(idx + 1) * 784]
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
/// Reads the following based on the dir: `train-imgs.npz`, `train-labels.npz`, `classmap.json`
///
/// Not supported on WASM file I/O requires the native target, future me can
/// brain up how to deal with files in http dynamically. For now whatever.
pub fn try_load(dir: &str) -> Result<LoadedDataset, String> {
    #[cfg(target_arch = "wasm32")]
    return Err("file loading is not supported on the web build".into());

    #[cfg(not(target_arch = "wasm32"))]
    {
        let imgs_path = format!("{dir}/train-imgs.npz");
        let labels_path = format!("{dir}/train-labels.npz");
        let classmap_path = format!("{dir}/classmap.json");

        let imgs_raw = read_npz_entry(&imgs_path)?;
        let (imgs_dtype, imgs_shape, imgs_offset) = parse_npy_header(&imgs_raw, &imgs_path)?;
        if imgs_dtype != "|u1" {
            return Err(format!(
                "{imgs_path}: expected |u1 images, got {imgs_dtype}"
            ));
        }
        if imgs_shape.len() != 3 || imgs_shape[1] != 28 || imgs_shape[2] != 28 {
            return Err(format!(
                "{imgs_path}: expected (N,28,28), got {imgs_shape:?}"
            ));
        }
        let n_images = imgs_shape[0];
        let img_data = imgs_raw[imgs_offset..].to_vec();

        let lbl_raw = read_npz_entry(&labels_path)?;
        let (lbl_dtype, lbl_shape, lbl_offset) = parse_npy_header(&lbl_raw, &labels_path)?;
        if lbl_shape.len() != 1 || lbl_shape[0] != n_images {
            return Err(format!(
                "{labels_path}: expected ({n_images},), got {lbl_shape:?}"
            ));
        }
        let lbl_data = &lbl_raw[lbl_offset..];
        let labels: Vec<u32> = match lbl_dtype.as_str() {
            "<u2" | "|u2" => (0..n_images)
                .map(|i| u16::from_le_bytes([lbl_data[i * 2], lbl_data[i * 2 + 1]]) as u32)
                .collect(),
            _ => lbl_data[..n_images].iter().map(|&b| b as u32).collect(),
        };

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
        })
    }
}

//TODO: A lot of this is duplicated from `ma` probably time for a shared lib crate.
fn read_npz_entry(path: &str) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("cannot open {path}: {e}"))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("{path}: not a zip archive: {e}"))?;
    let name = {
        let e = zip
            .by_index(0)
            .map_err(|e| format!("{path}: empty archive: {e}"))?;
        e.name().to_owned()
    };
    let mut entry = zip
        .by_name(&name)
        .map_err(|e| format!("{path}: cannot open entry: {e}"))?;
    let mut raw = Vec::new();
    entry
        .read_to_end(&mut raw)
        .map_err(|e| format!("{path}: read error: {e}"))?;
    Ok(raw)
}

fn parse_npy_header(raw: &[u8], path: &str) -> Result<(String, Vec<usize>, usize), String> {
    if raw.len() < 10 || &raw[..6] != b"\x93NUMPY" {
        return Err(format!("{path}: not a valid NPY file"));
    }
    let hlen = u16::from_le_bytes([raw[8], raw[9]]) as usize;
    let data_offset = 10 + hlen;
    if raw.len() < data_offset {
        return Err(format!("{path}: truncated header"));
    }
    let header = std::str::from_utf8(&raw[10..data_offset])
        .map_err(|_| format!("{path}: header is not UTF-8"))?;

    let dtype = npy_field(header, "descr")
        .ok_or_else(|| format!("{path}: missing 'descr' in NPY header"))?;
    let shape_str = npy_field(header, "shape")
        .ok_or_else(|| format!("{path}: missing 'shape' in NPY header"))?;
    let shape =
        npy_shape(&shape_str).ok_or_else(|| format!("{path}: cannot parse shape '{shape_str}'"))?;

    Ok((dtype, shape, data_offset))
}

fn npy_field(header: &str, key: &str) -> Option<String> {
    let needle = format!("'{key}':");
    let start = header.find(&needle)? + needle.len();
    let rest = header[start..].trim_start();
    let value = if rest.starts_with('(') {
        let end = rest.find(')')?;
        rest[..=end].to_owned()
    } else if let Some(inner) = rest.strip_prefix('\'') {
        let end = inner.find('\'')?;
        inner[..end].to_owned()
    } else {
        let end = rest.find([',', '}', '\n']).unwrap_or(rest.len());
        rest[..end].trim().to_owned()
    };
    Some(value)
}

fn npy_shape(s: &str) -> Option<Vec<usize>> {
    let inner = s.trim().strip_prefix('(')?.strip_suffix(')')?;
    if inner.trim().is_empty() {
        return Some(vec![]);
    }
    inner
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<usize>().ok())
        .collect()
}

/// Return the Unicode standard name for `ch` in title case (e.g. "Exclamation
/// Mark"), or an empty string if the codepoint has no assigned name.
fn char_unicode_name(ch: char) -> String {
    unicode_names2::name(ch)
        .map(|n| {
            // The name comes back as ALL-CAPS; convert to Title Case for readability.
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

/// Draw a single 28x28 image as colored rects in the given egui ui target.
///
/// Pixels with high values are displayed as dark; near-zero pixels are skipped
/// for performance and left as white.
fn draw_image(ui: &mut egui::Ui, pixels: &[u8], scale: f32) -> egui::Response {
    let size = egui::vec2(28.0 * scale, 28.0 * scale);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, egui::Color32::WHITE);

        for row in 0..28usize {
            for col in 0..28usize {
                let v = pixels[row * 28 + col];
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

/// Bevy system that renders the Data Viewer egui window.
pub fn data_viewer_window(
    mut contexts: EguiContexts,
    query: Query<Entity, With<ShowDataViewer>>,
    mut state: ResMut<DataViewerState>,
    mut commands: Commands,
) -> Result {
    if query.is_empty() {
        return Ok(());
    }

    let mut open = true;

    egui::Window::new("Data Viewer")
        .open(&mut open)
        .default_size([920.0, 580.0])
        .resizable(true)
        .show(contexts.ctx_mut()?, |ui| {
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
                                "Loaded {} images, {} classes from {dir}",
                                ds.n_images, ds.n_classes
                            );
                            state.selected_class = 0;
                            state.dataset = Some(ds);
                        }
                        Err(e) => {
                            state.status = format!("Error: {e}");
                            state.dataset = None;
                        }
                    }
                }

                // Browse button (native only) opens the egui-native folder picker.
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Browse...").clicked() {
                    use std::path::PathBuf;
                    let start = PathBuf::from(&state.dir);
                    state.file_dialog = FileDialog::new().initial_directory(start);
                    state.file_dialog.pick_directory();
                }

                ui.add_space(8.0);

                // TODO: I'm not sure this is useful to keep its janky and I'm sick of the resize logic
                ui.label("Zoom:");
                ui.add(egui::Slider::new(&mut state.px_scale, 1.0..=6.0).step_by(0.5));
            });

            ui.label(egui::RichText::new(&state.status).weak().italics().small());
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

                    let ds = state.dataset.as_ref().unwrap();

                    let matching: Vec<usize> = (0..ds.n_classes)
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

                    let mut new_sel = selected;
                    egui::ScrollArea::vertical()
                        .id_salt("class_list")
                        .max_height(440.0)
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
                });

                ui.separator();

                ui.vertical(|ui| {
                    let ds = state.dataset.as_ref().unwrap();
                    let sel = state.selected_class;
                    let ch = ds.class_char(sel);
                    let count = ds.class_count(sel);
                    let img_size = 28.0 * scale;

                    let ch_name = char_unicode_name(ch);
                    let header = if ch_name.is_empty() {
                        format!("Class {sel} = '{ch}'  ({count} samples)")
                    } else {
                        format!("Class {sel} = '{ch}' {ch_name}  ({count} samples)")
                    };
                    ui.label(egui::RichText::new(header).strong());
                    ui.add_space(4.0);

                    const RIGHT_W: f32 = 920.0 - LEFT_W - 12.0;
                    let per_row = ((RIGHT_W + GRID_GAP) / (img_size + GRID_GAP)).floor() as usize;
                    let per_row = per_row.max(1);

                    let indices = ds.by_class.get(sel).map(|v| v.as_slice()).unwrap_or(&[]);
                    let show_n = indices.len().min(120);

                    egui::ScrollArea::vertical()
                        .id_salt("img_grid")
                        .show(ui, |ui| {
                            for chunk in indices[..show_n].chunks(per_row) {
                                ui.horizontal(|ui| {
                                    for &img_idx in chunk {
                                        let px = ds.image(img_idx);
                                        let resp = draw_image(ui, px, scale);
                                        resp.on_hover_text(format!("sample {img_idx}"));
                                        ui.add_space(GRID_GAP);
                                    }
                                });
                                ui.add_space(GRID_GAP);
                            }

                            if show_n < indices.len() {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "… {} more samples not shown",
                                        indices.len() - show_n
                                    ))
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
