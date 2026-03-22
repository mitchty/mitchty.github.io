use std::{io::Read, sync::Arc};

use burn::{
    data::{
        dataloader::batcher::Batcher,
        dataset::{Dataset, vision::MnistItem},
    },
    prelude::*,
};

/// A single 28x28 grayscale image with a class label.
///
/// Replaces burn's `MnistItem` which uses `label: u8` so I can use something
/// more useful for Japanese/Kanji. Otherwise I'm capped at 255 classes which
/// won't even scratch the Joyo Kanji. JIS X 0208 is amost 7k characters so...
/// yeah
#[derive(Clone, Debug)]
pub struct DataItem {
    pub image: [[f32; 28]; 28],
    /// Class index, supports up to 4 billion classes, should be "good enough".
    pub label: u32,
}

/// Parse a single csv record, header not incldued into an MnistItem.
fn parse_record(record: &csv::StringRecord, row: usize) -> MnistItem {
    let label: u8 = record[0]
        .parse()
        .unwrap_or_else(|e| panic!("failed to parse label at row {row}: {e}"));

    let mut image = [[0f32; 28]; 28];
    for (i, pixel) in record.iter().skip(1).enumerate() {
        let val: f32 = pixel
            .parse::<u8>()
            .unwrap_or_else(|e| panic!("failed to parse pixel {i} at row {row}: {e}"))
            as f32;
        image[i / 28][i % 28] = val;
    }

    MnistItem { image, label }
}

/// Load every labelled row from a Kaggle mnist csv into memory first
///
/// csv layout appears to be like so: label,pixel0,pixel1,...,pixel783
//TODO: marked for deletion kaboom
fn load_kaggle_csv(path: &str) -> Vec<MnistItem> {
    let mut reader = csv::Reader::from_path(path)
        .unwrap_or_else(|e| panic!("failed to open Kaggle CSV at {path}: {e}"));

    reader
        .records()
        .enumerate()
        .map(|(i, res)| {
            let record = res.unwrap_or_else(|e| panic!("failed to read row {i} in {path}: {e}"));
            parse_record(&record, i)
        })
        .collect()
}

/// Load a single item from a kaggle mnist csv file by row index.
///
/// Note: This is only for the current infer logic. Not likely to survive real world usage.
// TODO: kaboom
pub fn load_kaggle_item(path: &str, index: usize) -> MnistItem {
    let mut reader = csv::Reader::from_path(path)
        .unwrap_or_else(|e| panic!("failed to open kaggle csv at {path}: {e}"));

    let record = reader
        .records()
        .nth(index)
        .unwrap_or_else(|| panic!("index {index} out of range in {path}"))
        .unwrap_or_else(|e| panic!("failed to read record {index} from {path}: {e}"));

    parse_record(&record, index)
}

/// An in-memory burn `Dataset` backed by a kaggle mnist csv file.
///
/// For "some" mnist kaggle datasets shipped this way `split()` lets you divide
/// it into train and validation subsets without copying the underlying data.
/// Which is nice.
#[derive(Clone)]
pub struct KaggleMnistDataset {
    items: Arc<Vec<MnistItem>>,
    /// This is what provides the underlying split() behavior
    start: usize,
    end: usize,
}

impl KaggleMnistDataset {
    pub fn from_csv(path: &str) -> Self {
        let items = Arc::new(load_kaggle_csv(path));
        let end = items.len();
        Self {
            items,
            start: 0,
            end,
        }
    }

    /// Split the csv into (train, validation) tuples.
    ///
    /// `train_fraction` must be within range of `(0.0, 1.0)`. The first tuple
    /// portion becomes the training set; the remainder becomes the validation
    /// set. Both share the same underlying `Arc<Vec<MnistItem>>` so there is no
    /// data duplication/wasted rams.
    // TODO: I should make the validation subset random... somehow
    pub fn split(self, train_fraction: f64) -> (Self, Self) {
        // future mitch is being stupid somehow... again...
        assert!(
            (0.0..1.0).contains(&train_fraction),
            "train_fraction must be in (0.0, 1.0), got {train_fraction}"
        );
        let total = self.end - self.start;
        let split = self.start + (total as f64 * train_fraction) as usize;
        let train = Self {
            items: self.items.clone(),
            start: self.start,
            end: split,
        };
        let val = Self {
            items: self.items,
            start: split,
            end: self.end,
        };
        (train, val)
    }
}

impl Dataset<DataItem> for KaggleMnistDataset {
    fn get(&self, index: usize) -> Option<DataItem> {
        let abs = self.start + index;
        if abs < self.end {
            let item = &self.items[abs];
            Some(DataItem {
                image: item.image,
                label: item.label as u32,
            })
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.end - self.start
    }
}

/// Parsed NPY v1 header.
struct NpyHeader {
    #[allow(dead_code)]
    dtype: String,
    shape: Vec<usize>,
    /// Byte offset at which raw array data actually begins.
    data_offset: usize,
}

/// Parse a numpy v1.x NPY file.
///
/// Validates magic, major version, dtype, and shape dimensionality. Panics with
/// a message that includes `path` so future mitch can figure out the problem
/// with that file. Or go drink one of the two.
fn parse_npy(raw: &[u8], path: &str) -> NpyHeader {
    assert!(
        raw.len() >= 10,
        "npz entry in {path} is too short to be a valid NPY file {} bytes",
        raw.len()
    );
    assert_eq!(
        &raw[..6],
        b"\x93NUMPY",
        "{path} is not a valid NPY file bad magic bytes"
    );
    let major = raw[6];
    assert_eq!(
        major, 1,
        "{path} only NPY v1.x is supported not major version {major}"
    );
    // NOTE minor version in raw[7] 0 or 1 for v1.x, doesn't affect the underlying data layout

    let header_len = u16::from_le_bytes([raw[8], raw[9]]) as usize;
    let data_offset = 10 + header_len;
    assert!(
        raw.len() >= data_offset,
        "{path} NPY header claims {header_len} header bytes but file is only {} bytes, someones lying",
        raw.len()
    );

    let header_str = std::str::from_utf8(&raw[10..data_offset])
        .unwrap_or_else(|_| panic!("{path} NPY header is not valid UTF-8"));

    let dtype = extract_header_field(header_str, "descr")
        .unwrap_or_else(|| panic!("{path} NPY header missing 'descr' field:\n  {header_str}"));

    // Dtype is validated by the caller, not here, since labels may be
    // '|u1' u8, K49/KMNIST or '<u2' u16 LE, large-class ETL datasets

    let shape_str = extract_header_field(header_str, "shape")
        .unwrap_or_else(|| panic!("{path} NPY header missing 'shape' field:\n  {header_str}"));

    let shape = parse_shape(&shape_str)
        .unwrap_or_else(|| panic!("{path} could not parse shape tuple '{shape_str}'"));

    NpyHeader {
        dtype,
        shape,
        data_offset,
    }
}

/// Pull the value for a key out of a Python-dict-style NPY header string.
///
/// Handles both `'key': value` and `'key': 'value'` (quoted) forms.
/// Returns the inner string without surrounding quotes.
fn extract_header_field(header: &str, key: &str) -> Option<String> {
    // Look for 'key':
    let needle = format!("'{key}':");
    let start = header.find(&needle)? + needle.len();
    let rest = header[start..].trim_start();

    // TODO: use a real parsing library lazy
    let value = if rest.starts_with('(') {
        // We got a tuple grab everything up to the matching ')'
        let end = rest.find(')')?;
        rest[..=end].trim().to_owned()
    } else if let Some(inner) = rest.strip_prefix('\'') {
        let end = inner.find('\'')?;
        inner[..end].to_owned()
    } else {
        // Bare word (True, False, number)
        let end = rest.find([',', '}', '\n']).unwrap_or(rest.len());
        rest[..end].trim().to_owned()
    };
    Some(value)
}

/// Parse a Python tuple string like `(270912,)` or `(2385, 28, 28)` to `Vec<usize>`.
fn parse_shape(s: &str) -> Option<Vec<usize>> {
    let inner = s.trim().strip_prefix('(')?.strip_suffix(')')?;
    if inner.trim().is_empty() {
        return Some(vec![]);
    }
    inner
        .split(',')
        .map(|part| part.trim())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<usize>().ok())
        .collect()
}

/// Read all bytes from the first entry in an npz, note its just a zip file.
fn read_npz_first_entry(path: &str) -> Vec<u8> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("cannot open npz {path} {e}"));
    let mut zip = zip::ZipArchive::new(file)
        .unwrap_or_else(|e| panic!("{path} cannot read as zip archive {e}"));
    let entry_name = zip
        .by_index(0)
        .map(|e| e.name().to_owned())
        .unwrap_or_else(|e| panic!("{path} no entries in zip {e}"));
    let mut entry = zip
        .by_name(&entry_name)
        .unwrap_or_else(|e| panic!("{path} cannot open zip entry '{entry_name}' {e}"));
    let mut raw = Vec::new();
    entry
        .read_to_end(&mut raw)
        .unwrap_or_else(|e| panic!("{path} error reading zip entry '{entry_name}' {e}"));
    raw
}

/// In-memory burn `Dataset` backed by a pair of npz files.
///
/// - `imgs_path`   = npz containing a `|u1` array of shape (N, 28, 28)
/// - `labels_path` = npz containing a `|u1` u8 or `<u2` u16 LE array of shape (N,)
///
/// K49 / KMNIST files use `|u1` labels (<=255 classes).
/// ETL files written by `ma convert` for >255 classes use `<u2` labels.
/// Both are detected automatically from the NPY header dtype so no bigs.
///
/// `num_classes()` returns `max_label + 1` and should be passed to `ModelConfig`.
#[derive(Clone)]
pub struct NpzDataset {
    items: Arc<Vec<DataItem>>,
    start: usize,
    end: usize,
}

impl NpzDataset {
    pub fn from_npz(imgs_path: &str, labels_path: &str) -> Self {
        let imgs_raw = read_npz_first_entry(imgs_path);
        let labels_raw = read_npz_first_entry(labels_path);

        let imgs_hdr = parse_npy(&imgs_raw, imgs_path);
        let labels_hdr = parse_npy(&labels_raw, labels_path);

        // Images must always be uint8.
        assert_eq!(
            imgs_hdr.dtype, "|u1",
            "{imgs_path} image array must be uint8 '|u1' but got dtype '{}' did you pass an unsupported datatype float32 or int32 image file by mistake?",
            imgs_hdr.dtype
        );

        // Validate images must be 3dim with last two dims 28x28
        assert_eq!(
            imgs_hdr.shape.len(),
            3,
            "{imgs_path} expected a 3dim image array (N, 28, 28) but got shape {:?} did you swap --train-imgs and --train-labels again dum dum?",
            imgs_hdr.shape
        );
        assert_eq!(
            imgs_hdr.shape[1], 28,
            "{imgs_path} image height must be 28, got {}",
            imgs_hdr.shape[1]
        );
        assert_eq!(
            imgs_hdr.shape[2], 28,
            "{imgs_path} image width must be 28, got {}",
            imgs_hdr.shape[2]
        );

        // Validate labels: must be 1-D.
        assert_eq!(
            labels_hdr.shape.len(),
            1,
            "{labels_path}: expected a 1dim label array (N,) but got shape {:?} did you swap --train-imgs and --train-labels again dum dum?",
            labels_hdr.shape
        );

        let n_imgs = imgs_hdr.shape[0];
        let n_labels = labels_hdr.shape[0];
        assert_eq!(
            n_imgs, n_labels,
            "image count ({n_imgs} from {imgs_path}) != label count ({n_labels} from {labels_path})"
        );

        let img_data = &imgs_raw[imgs_hdr.data_offset..];
        let label_data = &labels_raw[labels_hdr.data_offset..];

        // Read labels as u32, supporting both |u1 1 byte and <u2 2 bytes LE chungus datasets
        let labels_u32: Vec<u32> = match labels_hdr.dtype.as_str() {
            "<u2" | "|u2" => {
                // 2-byte little-endian u16 labels ETL large-class datasets of like 6k or more
                assert_eq!(
                    label_data.len(),
                    n_labels * 2,
                    "{labels_path}: expected {n_labels}x2 bytes for <u2 labels, got {}",
                    label_data.len()
                );
                (0..n_labels)
                    .map(|i| u16::from_le_bytes([label_data[i * 2], label_data[i * 2 + 1]]) as u32)
                    .collect()
            }
            _ => {
                // |u1 or anything else = 1 byte per label K49, KMNIST, MNIST style datasets aka small af datasets
                assert_eq!(
                    label_data.len(),
                    n_labels,
                    "{labels_path}: expected {n_labels} bytes for |u1 labels, got {}",
                    label_data.len()
                );
                label_data[..n_labels].iter().map(|&b| b as u32).collect()
            }
        };

        let items: Vec<DataItem> = (0..n_imgs)
            .map(|i| {
                let mut image = [[0f32; 28]; 28];
                let offset = i * 28 * 28;
                for row in 0..28 {
                    for col in 0..28 {
                        image[row][col] = img_data[offset + row * 28 + col] as f32;
                    }
                }
                DataItem {
                    image,
                    label: labels_u32[i],
                }
            })
            .collect();

        let end = items.len();
        Self {
            items: Arc::new(items),
            start: 0,
            end,
        }
    }

    /// Returns the number of distinct classes `max_label + 1`
    pub fn num_classes(&self) -> usize {
        self.items[self.start..self.end]
            .iter()
            .map(|it| it.label as usize)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0)
    }
}

impl Dataset<DataItem> for NpzDataset {
    fn get(&self, index: usize) -> Option<DataItem> {
        let abs = self.start + index;
        if abs < self.end {
            Some(self.items[abs].clone())
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.end - self.start
    }
}

/// A dataset formed by concatenating multiple `NpzDataset`s end-to-end like a heretic.
///
/// Burn's `Dataset::get` takes a flat index so this maps it across the inner
/// datasets in order. All inner datasets must use a **compatible label space**
/// it is the caller's responsibility to ensure that label `N` means the same
/// character in every dataset. Use the `--chars` flag in `ma convert` with the
/// same character list and order for all conversions in that case dum dum.
#[derive(Clone)]
pub struct ConcatNpzDataset {
    inner: Vec<NpzDataset>,
    /// Cumulative lengths `offsets[i]` = total items before `inner[i]`
    offsets: Vec<usize>,
    total: usize,
}

impl ConcatNpzDataset {
    /// Build from parallel slices of image-npz paths and label-npz paths.
    ///
    /// `imgs` and `labels` must have the same length and be ordered the same
    /// way (element `i` of `imgs` pairs with element `i` of `labels`).
    pub fn from_pairs(imgs: &[&str], labels: &[&str]) -> Self {
        assert_eq!(
            imgs.len(),
            labels.len(),
            "imgs and labels slices must have the same length"
        );
        assert!(!imgs.is_empty(), "must supply at least one npz pair");

        let inner: Vec<NpzDataset> = imgs
            .iter()
            .zip(labels.iter())
            .map(|(i, l)| NpzDataset::from_npz(i, l))
            .collect();

        let mut offsets = Vec::with_capacity(inner.len());
        let mut running = 0usize;
        for ds in &inner {
            offsets.push(running);
            running += ds.len();
        }

        Self {
            inner,
            offsets,
            total: running,
        }
    }

    /// Returns `max_label + 1` across all constituent datasets.
    pub fn num_classes(&self) -> usize {
        self.inner
            .iter()
            .map(|ds| ds.num_classes())
            .max()
            .unwrap_or(0)
    }
}

impl Dataset<DataItem> for ConcatNpzDataset {
    fn get(&self, index: usize) -> Option<DataItem> {
        if index >= self.total {
            return None;
        }
        let ds_idx = self
            .offsets
            .partition_point(|&off| off <= index)
            .saturating_sub(1);
        let local = index - self.offsets[ds_idx];
        self.inner[ds_idx].get(local)
    }

    fn len(&self) -> usize {
        self.total
    }
}

/// Apply random geometric augmentation to a 28x28 image for training.
///
/// Transformation parameters sampled each call:
/// - Rotation:    +-15degrees
/// - Translation: +02 pixels in x and y
/// - Scale:       0.85 <-> 1.15
///
/// Uses inverse-map bilinear sampling so every output pixel is a smooth
/// weighted average of its source neighbors. Out-of-bounds source pixels are
/// treated as background.
pub fn augment_image(
    image: &[[f32; 28]; 28],
    rng: &mut (impl rand::RngExt + ?Sized),
) -> [[f32; 28]; 28] {
    let sz = 28usize;
    let cx = (sz as f32 - 1.0) / 2.0;
    let cy = cx;

    let angle = (rng.random::<f32>() - 0.5) * 30.0_f32.to_radians(); // ±15°
    let tx = (rng.random::<f32>() - 0.5) * 4.0; // ±2 px
    let ty = (rng.random::<f32>() - 0.5) * 4.0; // ±2 px
    let scale = 0.85 + rng.random::<f32>() * 0.30; // 0.85 – 1.15

    let cos_a = angle.cos();
    let sin_a = angle.sin();

    let sample = |xi: isize, yi: isize| -> f32 {
        if xi >= 0 && xi < sz as isize && yi >= 0 && yi < sz as isize {
            image[yi as usize][xi as usize]
        } else {
            0.0
        }
    };

    // This is just glorified bilinear interpolation. Also I really wanted to
    // use a shader to do all this.
    let mut out = [[0f32; 28]; 28];
    for (y, _) in out.clone().iter().enumerate().take(sz) {
        for (x, _) in out.clone().iter().enumerate().take(sz) {
            let dx = x as f32 - cx - tx;
            let dy = y as f32 - cy - ty;
            let src_x = (dx * cos_a + dy * sin_a) / scale + cx;
            let src_y = (-dx * sin_a + dy * cos_a) / scale + cy;

            let x0 = src_x.floor() as isize;
            let y0 = src_y.floor() as isize;
            let fx = src_x - x0 as f32;
            let fy = src_y - y0 as f32;

            out[y][x] = (1.0 - fx) * (1.0 - fy) * sample(x0, y0)
                + fx * (1.0 - fy) * sample(x0 + 1, y0)
                + (1.0 - fx) * fy * sample(x0, y0 + 1)
                + fx * fy * sample(x0 + 1, y0 + 1);
        }
    }
    out
}

/// Normalization statistics used to scale pixel values during batching.
///
/// Must match between training and inference or the model will produce
/// garbage. The values are saved to `config.json` via `TrainingConfig` and
/// loaded by the inference engine so they stay in sync automatically.
///
/// K49 defaults (computed from k49-train-imgs.npz):  mean = 0.1793, std = 0.3416
/// MNIST defaults (PyTorch example):                  mean = 0.1307, std = 0.3081
// TODO: I need to make all the statistics dynamic.
#[derive(Clone, Copy, Debug)]
pub struct NormStats {
    pub mean: f32,
    pub std: f32,
}

impl NormStats {
    pub const K49: Self = Self {
        mean: 0.1793,
        std: 0.3416,
    };
    /// Original MNIST statistics kept for reference / fallback cause this all
    /// evolved from burn example code.
    #[allow(dead_code)]
    pub const MNIST: Self = Self {
        mean: 0.1307,
        std: 0.3081,
    };
}

/// Batcher that normalizes raw u8 pixel data and assembles burn tensors.
///
/// Construct with `MnistBatcher::new(stats)` rather than `Default` so that
/// the normalization constants are explicit and match what was recorded in
/// `config.json` at training time.
///
/// Call `.with_augment(true)` to enable per-image geometric augmentation
/// (rotation, translation, scale) use this for the training dataloader only;
/// keep it off for validation so metrics are only computed on clean images.
#[derive(Clone)]
pub struct MnistBatcher {
    stats: NormStats,
    augment: bool,
}

impl MnistBatcher {
    pub fn new(stats: NormStats) -> Self {
        Self {
            stats,
            augment: false,
        }
    }

    pub fn with_augment(mut self, augment: bool) -> Self {
        self.augment = augment;
        self
    }
}

impl Default for MnistBatcher {
    fn default() -> Self {
        Self::new(NormStats::K49)
    }
}

#[derive(Clone, Debug)]
pub struct MnistBatch<B: Backend> {
    pub images: Tensor<B, 3>,
    pub targets: Tensor<B, 1, Int>,
}

impl<B: Backend> Batcher<B, DataItem, MnistBatch<B>> for MnistBatcher {
    fn batch(&self, items: Vec<DataItem>, device: &B::Device) -> MnistBatch<B> {
        let mean = self.stats.mean;
        let std = self.stats.std;
        let augment = self.augment;
        let images = items
            .iter()
            .map(|item| {
                if augment {
                    let mut rng = rand::rng();
                    augment_image(&item.image, &mut rng)
                } else {
                    item.image
                }
            })
            .map(|image| TensorData::from(image).convert::<B::FloatElem>())
            .map(|data| Tensor::<B, 2>::from_data(data, device))
            .map(|tensor| tensor.reshape([1, 28, 28]))
            .map(|tensor| ((tensor / 255) - mean) / std)
            .collect();

        let targets = items
            .iter()
            .map(|item| {
                Tensor::<B, 1, Int>::from_data([(item.label as i64).elem::<B::IntElem>()], device)
            })
            .collect();

        let images = Tensor::cat(images, 0);
        let targets = Tensor::cat(targets, 0);

        MnistBatch { images, targets }
    }
}
