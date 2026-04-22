use std::sync::Arc;

use burn::{
    data::{dataloader::batcher::Batcher, dataset::Dataset},
    prelude::*,
};

/// A single grayscale image with a class label.
///
/// Images are stored as a flat row-major `Vec<u8>` with explicit `width` and
/// `height` so the dataset can hold any resolution - native ETL sizes (64x63,
/// 128x127, …) or anything you choose to resize to at training time.
///
/// Pixel values are raw `u8` in `[0, 255]` - the `MnistBatcher` converts to
/// normalized `f32` inline when building each batch, so no full-dataset f32
/// expansion ever happens.
///
/// Replaces burn's `MnistItem` to also support labels > 255 classes (Kanji
/// datasets need u32 - JIS X 0208 has ~7 k characters alone).
#[derive(Clone, Debug)]
pub struct DataItem {
    /// Flat row-major pixel buffer, length = `channels * width * height`, values in `[0, 255]`.
    /// Channel order matches the npz layout: channel 0 starts at index 0, channel 1 at H*W, etc.
    pub image: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Number of channels: 1 for grayscale, 3 for (original, Otsu, Sauvola).
    pub channels: usize,
    /// Class index, supports up to 4 billion classes.
    pub label: u32,
}

/// Read all bytes from the first entry in an npz file.
///
/// Delegates to `lib::npz::read_npz_first_entry`; panics on error with a
/// message that includes the path so debugging is not a nightmare.
fn read_npz_first_entry(path: &str) -> Vec<u8> {
    lib::npz::read_npz_first_entry(path).unwrap_or_else(|e| panic!("{e}"))
}

/// In-memory burn `Dataset` backed by a pair of npz files.
///
/// - `imgs_path`   = npz containing a `|u1` array of shape (N, H, W) - any H and W
/// - `labels_path` = npz containing a `|u1` u8 or `<u2` u16 LE array of shape (N,)
///
/// K49 / KMNIST files use `|u1` labels (<=255 classes).
/// ETL files written by `ma convert` for >255 classes use `<u2` labels.
/// Both are detected automatically from the NPY header dtype so no bigs.
///
/// `num_classes()` returns `max_label + 1` and should be passed to `ModelConfig`.
///
/// ## Memory layout
///
/// Pixels are stored as a single **contiguous `u8` slab** in the underlying
/// `Arc<Vec<u8>>` - identical to the raw NPZ bytes, no f32 expansion at load
/// time. `Dataset::get()` converts one image's worth of bytes to `Vec<f32>`
/// on demand, so only the currently-in-flight batch items are ever in f32 form.
///
/// For a 3-channel 64x63 dataset with N images this saves:
///
/// | Before | After |
/// |--------|-------|
/// | N x CxHxW x 4 bytes (f32 slab) + N x CxHxW x 1 byte (raw NPZ) alive simultaneously | N x CxHxW x 1 byte (u8 slab only) |
/// | Peak at load time ≈ 5x raw size | Peak at load time = 1x raw size |
#[derive(Clone)]
pub struct NpzDataset {
    /// Raw u8 pixel slab: N x channels x H x W bytes, C-order, same layout as the NPZ.
    pixels: Arc<Vec<u8>>,
    /// Decoded labels, one per image.
    labels: Arc<Vec<u32>>,
    /// Number of images in the full slab.
    #[allow(dead_code)]
    n: usize,
    channels: usize,
    height: usize,
    width: usize,
    /// First image index in this view of the slab (for future sub-range slicing).
    start: usize,
    /// One-past-last image index.
    end: usize,
}

impl NpzDataset {
    pub fn from_npz(imgs_path: &str, labels_path: &str) -> Self {
        let imgs_raw = read_npz_first_entry(imgs_path);
        let labels_raw = read_npz_first_entry(labels_path);

        let imgs_hdr =
            lib::npz::parse_npy_header(&imgs_raw, imgs_path).unwrap_or_else(|e| panic!("{e}"));
        let labels_hdr =
            lib::npz::parse_npy_header(&labels_raw, labels_path).unwrap_or_else(|e| panic!("{e}"));

        // Images must always be uint8.
        assert_eq!(
            imgs_hdr.dtype, "|u1",
            "{imgs_path}: image array must be uint8 '|u1' but got dtype '{}' - did you pass a float32 image file by mistake?",
            imgs_hdr.dtype
        );

        // Images must be 3-D (N, H, W) or 4-D (N, C, H, W).
        // The 4-D case arises from `ma convert --three-channel` which stores
        // (original, Otsu, Sauvola) as three channels.
        assert!(
            imgs_hdr.shape.len() == 3 || imgs_hdr.shape.len() == 4,
            "{imgs_path}: expected a 3-D (N,H,W) or 4-D (N,C,H,W) image array but got shape {:?}; did you swap --train-imgs and --train-labels?",
            imgs_hdr.shape
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

        // Extract channels, height, width from either (N,H,W) or (N,C,H,W).
        let (img_channels, img_h, img_w) = match imgs_hdr.shape.len() {
            3 => (1, imgs_hdr.shape[1], imgs_hdr.shape[2]),
            4 => (imgs_hdr.shape[1], imgs_hdr.shape[2], imgs_hdr.shape[3]),
            _ => unreachable!(),
        };

        // Decode labels; labels_raw can be dropped once done.
        let label_data = &labels_raw[labels_hdr.data_offset..];
        let labels_u32 = lib::npz::read_labels(label_data, n_labels, &labels_hdr.dtype);
        drop(labels_raw);

        // Strip the NPY header prefix from the decompressed buffer in-place so
        // the pixel slab can be wrapped in an Arc without a second allocation.
        //
        // `drain(0..data_offset)` drops the NPY magic, version bytes, and header
        // dict, leaving imgs_raw as just the pixel bytes. We then move it into
        // the Arc - no copy, no transient dual-buffer peak.
        let data_offset = imgs_hdr.data_offset;
        let ppi = img_channels * img_h * img_w;
        let expected_len = n_imgs * ppi;
        assert!(
            imgs_raw.len() >= data_offset + expected_len,
            "{imgs_path}: NPZ pixel data too short (expected {} bytes after header, got {})",
            expected_len,
            imgs_raw.len().saturating_sub(data_offset),
        );
        let mut pixels = imgs_raw;
        pixels.drain(0..data_offset); // drop NPY header prefix in-place
        pixels.truncate(expected_len); // drop any trailing padding bytes

        // imgs_raw was moved into `pixels` above; it is consumed here.
        Self {
            pixels: Arc::new(pixels),
            labels: Arc::new(labels_u32),
            n: n_imgs,
            channels: img_channels,
            height: img_h,
            width: img_w,
            start: 0,
            end: n_imgs,
        }
    }

    /// Returns the number of distinct classes `max_label + 1`
    pub fn num_classes(&self) -> usize {
        self.labels[self.start..self.end]
            .iter()
            .map(|&l| l as usize)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0)
    }

    /// Pixels per image (channels x height x width).
    fn ppi(&self) -> usize {
        self.channels * self.height * self.width
    }
}

impl Dataset<DataItem> for NpzDataset {
    fn get(&self, index: usize) -> Option<DataItem> {
        let abs = self.start + index;
        if abs >= self.end {
            return None;
        }
        let ppi = self.ppi();
        let offset = abs * ppi;
        // Copy the raw u8 pixels for this one image. The batcher converts to
        // normalized f32 inline while filling the batch buffer - 4x smaller
        // per-item allocation than returning Vec<f32>.
        let image: Vec<u8> = self.pixels[offset..offset + ppi].to_vec();
        Some(DataItem {
            image,
            channels: self.channels,
            width: self.width,
            height: self.height,
            label: self.labels[abs],
        })
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

/// Apply random geometric augmentation to a grayscale image for training.
///
/// Works on a flat row-major pixel buffer of any size - the caller supplies
/// `width` and `height` so the same function handles 28x28, 64x63, 128x127,
/// or whatever native resolution came out of the npz file.
///
/// Transformation parameters sampled each call:
/// - Rotation:    +/-15°
/// - Translation: +/-2 px in x and y (relative to image size)
/// - Scale:       0.85–1.15
///
/// Accepts raw `u8` pixels; bilinear sampling is performed in `f32` internally.
/// Returns a `Vec<f32>` (unnormalized, in `[0, 255]`) so the batcher can apply
/// its inline normalisation in the same pass as the augmented values.
///
/// Note: multi-channel augmentation (applying a shared transform across all
/// channels) is deferred to future work.
pub fn augment_image(
    image: &[u8],
    width: usize,
    height: usize,
    rng: &mut (impl rand::RngExt + ?Sized),
) -> Vec<f32> {
    let cx = (width as f32 - 1.0) / 2.0;
    let cy = (height as f32 - 1.0) / 2.0;

    let angle = (rng.random::<f32>() - 0.5) * 30.0_f32.to_radians(); // +/-15°
    let tx = (rng.random::<f32>() - 0.5) * 4.0; // +/-2 px
    let ty = (rng.random::<f32>() - 0.5) * 4.0; // +/-2 px
    let scale = 0.85 + rng.random::<f32>() * 0.30; // 0.85 – 1.15

    let cos_a = angle.cos();
    let sin_a = angle.sin();

    let sample = |xi: isize, yi: isize| -> f32 {
        if xi >= 0 && xi < width as isize && yi >= 0 && yi < height as isize {
            image[yi as usize * width + xi as usize] as f32
        } else {
            0.0
        }
    };

    let mut out = vec![0f32; width * height];
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - cx - tx;
            let dy = y as f32 - cy - ty;
            let src_x = (dx * cos_a + dy * sin_a) / scale + cx;
            let src_y = (-dx * sin_a + dy * cos_a) / scale + cy;

            let x0 = src_x.floor() as isize;
            let y0 = src_y.floor() as isize;
            let fx = src_x - x0 as f32;
            let fy = src_y - y0 as f32;

            out[y * width + x] = (1.0 - fx) * (1.0 - fy) * sample(x0, y0)
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
    /// Shape: `[batch, channels, height, width]`.
    /// `channels` is 1 for grayscale npz files and 3 for three-channel npz files.
    pub images: Tensor<B, 4>,
    pub targets: Tensor<B, 1, Int>,
}

impl<B: Backend> Batcher<B, DataItem, MnistBatch<B>> for MnistBatcher {
    fn batch(&self, items: Vec<DataItem>, device: &B::Device) -> MnistBatch<B> {
        let mean = self.stats.mean;
        let std = self.stats.std;
        let augment = self.augment;
        let n = items.len();

        // Pull shape from first item (all items in a batch share dimensions).
        let (c, h, w) = items
            .first()
            .map(|it| (it.channels, it.height, it.width))
            .unwrap_or((1, 1, 1));
        let ppi = c * h * w;

        // Build a single contiguous f32 buffer for the entire batch, with
        // normalisation applied inline. This replaces:
        //   - N x item.image.clone() (one Vec<f32> per item)
        //   - N x TensorData::new + Tensor::from_data (N individual uploads)
        //   - Tensor::cat(images, 0) (allocation + copy to merge N tensors)
        //
        // Now there is exactly one Vec<f32> allocation and one device upload
        // for the entire batch.
        let mut pixels_f32: Vec<f32> = Vec::with_capacity(n * ppi);

        let mut rng = rand::rng();
        let mut label_buf: Vec<i64> = Vec::with_capacity(n);

        for item in &items {
            // Augmentation is only applied to single-channel images for now;
            // multi-channel augmentation (shared spatial transform across
            // channels) is future work.
            //
            // augment_image takes &[u8] and returns Vec<f32> (in [0,255] range)
            // so normalisation is applied inline in both paths.
            if augment && item.channels == 1 {
                let aug = augment_image(&item.image, item.width, item.height, &mut rng);
                for v in aug {
                    pixels_f32.push((v / 255.0 - mean) / std);
                }
            } else {
                // Direct u8 -> normalized f32, no intermediate Vec<f32>.
                for &b in &item.image {
                    pixels_f32.push((b as f32 / 255.0 - mean) / std);
                }
            }
            label_buf.push(item.label as i64);
        }

        // Single device upload for the whole batch -> reshape to [B, C, H, W].
        let img_data = TensorData::new(pixels_f32, [n * ppi]).convert::<B::FloatElem>();
        let images: Tensor<B, 4> =
            Tensor::<B, 1>::from_data(img_data, device).reshape([n, c, h, w]);

        // Labels: one small allocation for the whole batch.
        let label_elems: Vec<B::IntElem> =
            label_buf.iter().map(|&l| l.elem::<B::IntElem>()).collect();
        let targets: Tensor<B, 1, Int> =
            Tensor::<B, 1, Int>::from_data(TensorData::new(label_elems, [n]), device);

        MnistBatch { images, targets }
    }
}
