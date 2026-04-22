//! Pure image-processing pipeline for ETL dataset cleanup/shenanigans in future?

/// Which thresholding algorithm to apply to the pre-filtered grayscale image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThresholdMode {
    /// No thresholding, show whatever has been applied up to here
    #[default]
    None,
    /// Global Otsu single threshold
    Otsu,
    /// Sauvola adaptive per-pixel threshold based on local mean + std-dev for
    /// when Otsu sucks.
    Sauvola,
    // TODO: Removeme
    /// Automatic quality-gated selection, this turned out to be a fart in church idea.
    ///
    /// Checks the bimodality coefficient of the pre-filtered image to try
    /// dynamically applying Otsu/Sauvola via:
    /// - BC > 0.555 try Otsu; accept if quality ge 0.4, else fall back.
    /// - BC ≤ 0.555 histogram is unimodal and Otsu will be ass for sure go straight to Sauvola threshold.
    ///
    /// Sauvola itself tries: SauvolaWide (w=11, k=0.2), SauvolaNarrow (w=7, k=0.15)
    /// Whichever gets a better quality "wins". This is a terrible way to use a CNN tho.
    Auto,
}

/// Which threshold algorithm [`ThresholdMode::Auto`] actually chose for a
/// given image. Returned in [`PipelineResult::auto_method`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoMethod {
    /// Otsu won fair and square
    Otsu,
    /// Sauvola wide window
    SauvolaWide,
    /// Sauvola with narrow window
    SauvolaNarrow,
}

impl std::fmt::Display for AutoMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutoMethod::Otsu => write!(f, "Otsu"),
            AutoMethod::SauvolaWide => write!(f, "Sauvola w=11 k=0.20"),
            AutoMethod::SauvolaNarrow => write!(f, "Sauvola w=7 k=0.15"),
        }
    }
}

/// All parameters that control a single run of [`run_pipeline`].
///
/// `Default` produces an all-off, pass-through configuration
/// (`ThresholdMode::None`, every filter disabled). Use [`ImagePipelineConfig::auto`]
/// to get a sensible preset for ETL dataset cleanup.
#[derive(Debug, Clone)]
pub struct ImagePipelineConfig {
    /// Apply a 2-D median filter before thresholding.
    pub median_prefilter: bool,
    /// Median filter neighborhood radius: 1 = 3x3, 2 = 5x5.
    pub median_radius: usize,
    /// Apply a Gaussian blur before thresholding.
    pub gaussian_prefilter: bool,
    /// Gaussian blur σ in pixels.
    pub gaussian_sigma: f32,
    /// Subtract a slowly-varying background illumination estimate.
    pub bg_normalize: bool,
    /// Background σ as a fraction of max(width, height).
    pub bg_sigma_scale: f32,
    /// Stretch the histogram to the full 0–255 range before thresholding.
    pub contrast_stretch: bool,
    /// Percentile of pixels to clip from each end of the histogram.
    pub contrast_clip_pct: f32,
    /// Thresholding algorithm (or `None` for grayscale pass-through).
    pub threshold_mode: ThresholdMode,
    /// Invert the threshold decision (keep dark pixels instead of bright).
    pub threshold_invert: bool,
    /// Sauvola local window half-size in pixels.
    pub sauvola_window: usize,
    /// Sauvola k sensitivity factor.
    pub sauvola_k: f32,
    /// Apply morphological opening (erode->dilate) to the binary output.
    pub morph_open: bool,
    /// Apply morphological closing (dilate->erode) to the binary output.
    pub morph_close: bool,
    /// Structuring-element radius for open/close (1=3x3, 2=5x5, 3=7x7).
    pub morph_radius: usize,
    /// Remove connected foreground components smaller than `min_component_size`.
    pub min_component: bool,
    /// Minimum blob area in pixels; smaller blobs are zeroed out.
    pub min_component_size: usize,
}

impl Default for ImagePipelineConfig {
    fn default() -> Self {
        Self {
            median_prefilter: false,
            median_radius: 1,
            gaussian_prefilter: false,
            gaussian_sigma: 1.0,
            bg_normalize: false,
            bg_sigma_scale: 0.15,
            contrast_stretch: false,
            contrast_clip_pct: 1.0,
            threshold_mode: ThresholdMode::None,
            threshold_invert: false,
            sauvola_window: 11,
            sauvola_k: 0.2,
            morph_open: false,
            morph_close: false,
            morph_radius: 1,
            min_component: false,
            min_component_size: 10,
        }
    }
}

impl ImagePipelineConfig {
    /// A sensible default for automatic ETL image cleanup?
    ///
    /// Enables [`ThresholdMode::Auto`] (BC-gated Otsu -> Sauvola fallback) with
    /// all other settings at their defaults (no pre- or post-filters). This is
    /// the recommended starting point for `ma convert --process-images`.
    pub fn auto() -> Self {
        Self {
            threshold_mode: ThresholdMode::Auto,
            ..Self::default()
        }
    }
}

/// Output of [`run_pipeline`].
#[derive(Debug)]
pub struct PipelineResult {
    /// Processed pixel buffer same `width x height` bytes as the input.
    /// Binary (0 / 255) when any threshold mode was active; grayscale otherwise.
    pub pixels: Vec<u8>,
    /// Quality score in `[0.0, 1.0]`. `None` when `threshold_mode` is `None`
    /// >= 0.60 = W | 0.30–0.59 = sus ⚠ | < 0.30 = ass
    pub quality: Option<f32>,
    /// Which method [`ThresholdMode::Auto`] chose. `None` for all other modes.
    pub auto_method: Option<AutoMethod>,
    /// Foreground density ratio: `count(255) / total`. `None` when
    /// `threshold_mode` is `None`.
    pub fdr: Option<f32>,
}

/// Compute all three training channels for a single grayscale image.
///
/// Returns `[original, otsu_binary, sauvola_binary]` three `width x height`
/// byte slices intended to be stacked as `[3, H, W]` for CNN training input.
///
/// | Channel | Content | When reliable |
/// |---------|---------|---------------|
/// | 0 | Raw grayscale copy | Always full continuous signal |
/// | 1 | Otsu binary (bright -> 255) | Clean bimodal histograms; may flood on noisy images |
/// | 2 | Sauvola binary (w=11, k=0.20, bright -> 255) | Uneven illumination; produces border halo on scan images |
///
/// Both binary channels use `invert = false` so bright areas  aka paper/background
/// map to 255 and dark areas ink, pencil map to 0 consistent with the data viewer's
/// default display convention of black on white drawing for classification.
///
/// The three views are complementary: the CNN can learn to weight each channel
/// according to what it reveals about a given image, without any hard-coded
/// selection heuristic like my dumbass tried with BC values.
pub fn compute_three_channels(pixels: &[u8], width: usize, height: usize) -> [Vec<u8>; 3] {
    let original = pixels.to_vec();
    let (otsu, _) = apply_otsu(pixels, false);
    let sauvola = sauvola_threshold(pixels, width, height, 11, 0.2, false);
    [original, otsu, sauvola]
}

/// Run the full image-processing pipeline on a single grayscale image.
///
/// `pixels` must be `width x height` bytes in row-major order. The returned
/// [`PipelineResult::pixels`] has the same dimensions.
// This is mostly for the gooey.
pub fn run_pipeline(
    pixels: &[u8],
    width: usize,
    height: usize,
    config: &ImagePipelineConfig,
) -> PipelineResult {
    // Each enabled stage replaces `current` with a new Vec<u8>.
    // For disabled stages `current` is unchanged. One initial copy is made
    // so that even the no-op path returns an owned buffer to save on rams a bit.
    let mut current: Vec<u8> = pixels.to_vec();

    if config.median_prefilter {
        current = median_filter_2d(&current, width, height, config.median_radius);
    }
    if config.gaussian_prefilter {
        current = gaussian_blur(&current, width, height, config.gaussian_sigma);
    }
    if config.bg_normalize {
        current = background_normalize(&current, width, height, config.bg_sigma_scale);
    }
    if config.contrast_stretch {
        current = contrast_stretch(&current, config.contrast_clip_pct);
    }

    let mut otsu_var: Option<f64> = None;
    let mut chosen_method: Option<AutoMethod> = None;

    match config.threshold_mode {
        ThresholdMode::None => {
            // Nop default image
        }
        ThresholdMode::Otsu => {
            let (px, var) = apply_otsu(&current, config.threshold_invert);
            otsu_var = Some(var);
            current = px;
        }
        ThresholdMode::Sauvola => {
            current = sauvola_threshold(
                &current,
                width,
                height,
                config.sauvola_window,
                config.sauvola_k,
                config.threshold_invert,
            );
        }
        ThresholdMode::Auto => {
            // BC-gated quality-checked fallback chain.
            //
            // Step 1 - check histogram bimodality.
            // Step 2 - if bimodal: try Otsu; accept if quality >= 0.4.
            // Step 3 - if Otsu rejected or was unimodal: try SauvolaWide first
            //           accept if quality >= 0.4.
            // Step 4 - if still below: try SauvolaNarrow; keep highest scorer of the two

            const AUTO_ACCEPT: f32 = 0.4;
            const BC_THRESHOLD: f32 = 0.555;

            let bc = bimodality_coefficient(&current);

            if bc > BC_THRESHOLD {
                // Bimodal Otsu has a good chance of working and not look like ass
                let (otsu_px, otsu_var_val) = apply_otsu(&current, config.threshold_invert);
                let q_otsu = pipeline_quality_score(&otsu_px, Some(otsu_var_val));

                if q_otsu >= AUTO_ACCEPT {
                    otsu_var = Some(otsu_var_val);
                    chosen_method = Some(AutoMethod::Otsu);
                    current = otsu_px;
                } else {
                    // Otsu looked like ass try Sauvola instead.
                    let mut best_px = otsu_px;
                    let mut best_q = q_otsu;
                    let mut best_method = AutoMethod::Otsu;
                    let mut best_is_otsu = true;

                    // SauvolaWide (w=11, k=0.2)
                    let sw = sauvola_threshold(
                        &current,
                        width,
                        height,
                        11,
                        0.2,
                        config.threshold_invert,
                    );
                    let qw = pipeline_quality_score(&sw, None);
                    if qw > best_q {
                        best_px = sw;
                        best_q = qw;
                        best_method = AutoMethod::SauvolaWide;
                        best_is_otsu = false;
                    }

                    // SauvolaNarrow (w=7, k=0.15) - only if still below acceptance.
                    if best_q < AUTO_ACCEPT {
                        let sn = sauvola_threshold(
                            &current,
                            width,
                            height,
                            7,
                            0.15,
                            config.threshold_invert,
                        );
                        let qn = pipeline_quality_score(&sn, None);
                        if qn > best_q {
                            best_px = sn;
                            best_method = AutoMethod::SauvolaNarrow;
                            best_is_otsu = false;
                        }
                    }

                    if best_is_otsu {
                        otsu_var = Some(otsu_var_val);
                    }
                    chosen_method = Some(best_method);
                    current = best_px;
                }
            } else {
                // Unimodal histogram, Otsu will be ass here too. Try SauvolaWide instead.
                let sw =
                    sauvola_threshold(&current, width, height, 11, 0.2, config.threshold_invert);
                let qw = pipeline_quality_score(&sw, None);
                let mut best_px = sw;
                let best_q = qw;
                let mut best_method = AutoMethod::SauvolaWide;

                if best_q < AUTO_ACCEPT {
                    let sn = sauvola_threshold(
                        &current,
                        width,
                        height,
                        7,
                        0.15,
                        config.threshold_invert,
                    );
                    let qn = pipeline_quality_score(&sn, None);
                    if qn > best_q {
                        best_px = sn;
                        best_method = AutoMethod::SauvolaNarrow;
                    }
                }

                chosen_method = Some(best_method);
                current = best_px;
            }
        }
    }

    let is_binary = config.threshold_mode != ThresholdMode::None;

    if config.morph_open && is_binary {
        current = morph_open(&current, width, height, config.morph_radius);
    }
    if config.morph_close && is_binary {
        current = morph_close(&current, width, height, config.morph_radius);
    }
    if config.min_component && is_binary {
        current = min_component_filter(&current, width, height, config.min_component_size);
    }

    let (quality, fdr) = if is_binary {
        let q = pipeline_quality_score(&current, otsu_var);
        let fg = current.iter().filter(|&&b| b == 255).count() as f32;
        let f = fg / current.len() as f32;
        (Some(q), Some(f))
    } else {
        (None, None)
    };

    PipelineResult {
        pixels: current,
        quality,
        auto_method: chosen_method,
        fdr,
    }
}

/// Blur a grayscale image with a Gaussian kernel using two separable 1-D passes.
///
/// The kernel radius is `ceil(3 x sigma)` pixels, giving negligible truncation
/// error. Border pixels are handled by clamping the sample coordinate to the
/// image edge (replicate-border mode).
pub fn gaussian_blur(pixels: &[u8], width: usize, height: usize, sigma: f32) -> Vec<u8> {
    let radius = (3.0 * sigma).ceil() as usize;
    let ksize = 2 * radius + 1;
    let mut kernel = vec![0.0f32; ksize];
    let s2 = 2.0 * sigma * sigma;
    let mut ksum = 0.0f32;
    for (i, v) in kernel.iter_mut().enumerate().take(ksize) {
        let x = i as f32 - radius as f32;
        *v = (-x * x / s2).exp();
        ksum += *v;
    }
    for v in &mut kernel {
        *v /= ksum;
    }

    // Horizontal pass first.
    let mut tmp = vec![0.0f32; width * height];
    for r in 0..height {
        for c in 0..width {
            let mut acc = 0.0f32;
            for (ki, &kv) in kernel.iter().enumerate() {
                let sc = (c as isize + ki as isize - radius as isize).clamp(0, width as isize - 1)
                    as usize;
                acc += pixels[r * width + sc] as f32 * kv;
            }
            tmp[r * width + c] = acc;
        }
    }

    // Vertical pass second.
    let mut out = vec![0u8; width * height];
    for r in 0..height {
        for c in 0..width {
            let mut acc = 0.0f32;
            for (ki, &kv) in kernel.iter().enumerate() {
                let sr = (r as isize + ki as isize - radius as isize).clamp(0, height as isize - 1)
                    as usize;
                acc += tmp[sr * width + c] * kv;
            }
            out[r * width + c] = acc.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Apply a 2-D median filter to an image.
///
/// `radius` controls the neighborhood size: 1 = 3x3, 2 = 5x5.
/// Border pixels use replicate-border sampling approach.
// This isn't all that useful for my needs will nuke it later
pub fn median_filter_2d(pixels: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let diameter = 2 * radius + 1;
    let n = diameter * diameter;
    let mut out = vec![0u8; width * height];
    let mut neighbors = vec![0u8; n];
    for r in 0..height {
        for c in 0..width {
            let mut k = 0usize;
            for dr in -(radius as isize)..=(radius as isize) {
                for dc in -(radius as isize)..=(radius as isize) {
                    let nr = (r as isize + dr).clamp(0, height as isize - 1) as usize;
                    let nc = (c as isize + dc).clamp(0, width as isize - 1) as usize;
                    neighbors[k] = pixels[nr * width + nc];
                    k += 1;
                }
            }
            neighbors.sort_unstable();
            out[r * width + c] = neighbors[n / 2];
        }
    }
    out
}

/// Binarize a grayscale image using Sauvola local adaptive threshold.
///
/// Foreach pixel `(x, y)`:
/// ```text
/// T(x,y) = μ(x,y) · [1 + k · (σ(x,y) / R − 1)]
/// ```
/// where μ and σ are the local mean and std-dev over a
/// `(2·window+1) x (2·window+1)` neighborhood, R = 128, and `k approx 0.2`.
/// Integral images make this O(N) regardless of window size.
///
/// Without `invert`: pixels >= T -> 255. With `invert`: pixels < T -> 255.
pub fn sauvola_threshold(
    pixels: &[u8],
    width: usize,
    height: usize,
    window: usize,
    k: f32,
    invert: bool,
) -> Vec<u8> {
    const R: f32 = 128.0;
    let half = window as isize;
    let stride = width + 1;

    let mut integral = vec![0i64; stride * (height + 1)];
    let mut integral_sq = vec![0i64; stride * (height + 1)];

    for row in 0..height {
        for col in 0..width {
            let v = pixels[row * width + col] as i64;
            let idx = (row + 1) * stride + (col + 1);
            let up = row * stride + (col + 1);
            let left = (row + 1) * stride + col;
            let ul = row * stride + col;
            integral[idx] = v + integral[up] + integral[left] - integral[ul];
            integral_sq[idx] = v * v + integral_sq[up] + integral_sq[left] - integral_sq[ul];
        }
    }

    let mut out = vec![0u8; width * height];
    for row in 0..height {
        for col in 0..width {
            let r0 = (row as isize - half).max(0) as usize;
            let r1 = (row as isize + half).min(height as isize - 1) as usize;
            let c0 = (col as isize - half).max(0) as usize;
            let c1 = (col as isize + half).min(width as isize - 1) as usize;

            let count = ((r1 - r0 + 1) * (c1 - c0 + 1)) as f32;
            let sum = (integral[(r1 + 1) * stride + (c1 + 1)]
                - integral[r0 * stride + (c1 + 1)]
                - integral[(r1 + 1) * stride + c0]
                + integral[r0 * stride + c0]) as f32;
            let sum_sq = (integral_sq[(r1 + 1) * stride + (c1 + 1)]
                - integral_sq[r0 * stride + (c1 + 1)]
                - integral_sq[(r1 + 1) * stride + c0]
                + integral_sq[r0 * stride + c0]) as f32;

            let mean = sum / count;
            let variance = (sum_sq / count - mean * mean).max(0.0);
            let std_dev = variance.sqrt();
            let threshold = mean * (1.0 + k * (std_dev / R - 1.0));

            let v = pixels[row * width + col];
            let foreground = v as f32 >= threshold;
            out[row * width + col] = if foreground ^ invert { 255 } else { 0 };
        }
    }
    out
}

/// Subtract a slowly-varying background illumination estimate.
///
/// A large Gaussian (σ = `max(w,h) x sigma_scale`) approximates the paper/desk
/// background seen in ETL data sts. Each pixel is normalized: `clamp(pixel x 128 / background, 0, 255)`.
pub fn background_normalize(
    pixels: &[u8],
    width: usize,
    height: usize,
    sigma_scale: f32,
) -> Vec<u8> {
    let sigma = (width.max(height) as f32 * sigma_scale).max(1.0);
    let bg = gaussian_blur(pixels, width, height, sigma);
    pixels
        .iter()
        .zip(bg.iter())
        .map(|(&p, &b)| {
            let b = b.max(1) as f32;
            (p as f32 * 128.0 / b).clamp(0.0, 255.0) as u8
        })
        .collect()
}

/// Stretch the histogram of a grayscale image to the full 0–255 range.
///
/// Pixels at/below `clip_pct` percentile map to 0; pixels at/above the `(100 −
/// clip_pct)`-th percentile map to 255. The interior is linear and looks a bit
/// like a bloom filter.
pub fn contrast_stretch(pixels: &[u8], clip_pct: f32) -> Vec<u8> {
    let mut hist = [0u32; 256];
    for &p in pixels {
        hist[p as usize] += 1;
    }
    let total = pixels.len();
    let clip = (total as f32 * clip_pct / 100.0).max(0.0) as u32;

    let mut lo = 0usize;
    let mut cumsum = 0u32;
    for (i, &h) in hist.iter().enumerate() {
        cumsum += h;
        if cumsum > clip {
            lo = i;
            break;
        }
    }

    let mut hi = 255usize;
    cumsum = 0;
    for i in (0..256).rev() {
        cumsum += hist[i];
        if cumsum > clip {
            hi = i;
            break;
        }
    }

    if hi <= lo {
        return pixels.to_vec();
    }
    let range = (hi - lo) as f32;
    pixels
        .iter()
        .map(|&p| {
            let v = p as usize;
            if v <= lo {
                0
            } else if v >= hi {
                255
            } else {
                ((v - lo) as f32 / range * 255.0).round() as u8
            }
        })
        .collect()
}

/// Morphological erosion on a binary (0 / 255) image.
///
/// A pixel stays 255 only if every pixel in the square `+/-radius` neighborhood
/// is also 255. Border pixels use replicate-border sampling.
pub fn morph_erode(pixels: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let mut out = vec![255u8; width * height];
    for r in 0..height {
        for c in 0..width {
            'nbr: for dr in -(radius as isize)..=(radius as isize) {
                for dc in -(radius as isize)..=(radius as isize) {
                    let nr = (r as isize + dr).clamp(0, height as isize - 1) as usize;
                    let nc = (c as isize + dc).clamp(0, width as isize - 1) as usize;
                    if pixels[nr * width + nc] == 0 {
                        out[r * width + c] = 0;
                        break 'nbr;
                    }
                }
            }
        }
    }
    out
}

/// Morphological dilation on a binary (0 / 255) image.
///
/// A pixel becomes 255 if any pixel in the square `+/-radius` neighborhood
/// is 255. Border pixels use replicate-border sampling.
pub fn morph_dilate(pixels: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height];
    for r in 0..height {
        for c in 0..width {
            'nbr: for dr in -(radius as isize)..=(radius as isize) {
                for dc in -(radius as isize)..=(radius as isize) {
                    let nr = (r as isize + dr).clamp(0, height as isize - 1) as usize;
                    let nc = (c as isize + dc).clamp(0, width as isize - 1) as usize;
                    if pixels[nr * width + nc] == 255 {
                        out[r * width + c] = 255;
                        break 'nbr;
                    }
                }
            }
        }
    }
    out
}

/// Morphological opening: erosion followed by dilation.
/// Removes small foreground blobs without altering larger features.
pub fn morph_open(pixels: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let eroded = morph_erode(pixels, width, height, radius);
    morph_dilate(&eroded, width, height, radius)
}

/// Morphological closing: dilation followed by erosion.
/// Fills small holes and gaps in foreground regions.
pub fn morph_close(pixels: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let dilated = morph_dilate(pixels, width, height, radius);
    morph_erode(&dilated, width, height, radius)
}

/// Remove connected foreground (255) components smaller than `min_size` pixels.
///
/// Uses BFS flood-fill with 8-connectivity. More surgical than morphological
/// opening: deletes entire isolated blobs without risk of eroding thin strokes.
pub fn min_component_filter(
    pixels: &[u8],
    width: usize,
    height: usize,
    min_size: usize,
) -> Vec<u8> {
    let mut out = pixels.to_vec();
    let mut visited = vec![false; width * height];
    let mut queue = std::collections::VecDeque::new();

    for start_r in 0..height {
        for start_c in 0..width {
            let idx = start_r * width + start_c;
            if visited[idx] || out[idx] == 0 {
                visited[idx] = true;
                continue;
            }

            let mut component: Vec<usize> = Vec::new();
            queue.clear();
            queue.push_back((start_r, start_c));
            visited[idx] = true;

            while let Some((r, c)) = queue.pop_front() {
                component.push(r * width + c);
                for dr in -1isize..=1 {
                    for dc in -1isize..=1 {
                        if dr == 0 && dc == 0 {
                            continue;
                        }
                        let nr = r as isize + dr;
                        let nc = c as isize + dc;
                        if nr < 0 || nr >= height as isize || nc < 0 || nc >= width as isize {
                            continue;
                        }
                        let nidx = nr as usize * width + nc as usize;
                        if !visited[nidx] && out[nidx] == 255 {
                            visited[nidx] = true;
                            queue.push_back((nr as usize, nc as usize));
                        }
                    }
                }
            }

            if component.len() < min_size {
                for idx in component {
                    out[idx] = 0;
                }
            }
        }
    }
    out
}

/// Compute the Otsu threshold for a grayscale pixel slice.
///
/// Returns `(threshold_byte, inter_class_variance)`. High variance (> 1 000)
/// indicates a cleanly bimodal histogram Otsu is reliable. Low variance
/// means the histogram is unimodal and the threshold is unreliable.
///
/// Returns `(0, 0.0)` for an empty slice.
pub fn otsu_threshold(pixels: &[u8]) -> (u8, f64) {
    let mut hist = [0u64; 256];
    for &p in pixels {
        hist[p as usize] += 1;
    }

    let total = pixels.len() as f64;
    if total == 0.0 {
        return (0, 0.0);
    }

    let global_mean: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum::<f64>()
        / total;

    let mut best_thresh = 0u8;
    let mut best_var = 0.0_f64;
    let mut w0 = 0.0_f64;
    let mut mean0_sum = 0.0_f64;

    for (t, &h) in hist.iter().enumerate() {
        let cnt = h as f64;
        w0 += cnt / total;
        mean0_sum += t as f64 * cnt / total;
        let w1 = 1.0 - w0;
        if w0 == 0.0 || w1 == 0.0 {
            continue;
        }

        let mean0 = mean0_sum / w0;
        let mean1 = (global_mean - mean0_sum * w0) / w1;
        let var_b = w0 * w1 * (mean0 - mean1) * (mean0 - mean1);
        if var_b > best_var {
            best_var = var_b;
            best_thresh = t as u8;
        }
    }

    (best_thresh, best_var)
}

/// Apply Otsu to a pixel slice.
///
/// Returns `(binarized_pixels, inter_class_variance)`.
/// Pixels **>= threshold** -> 255 when `invert` is false; **< threshold** -> 255 when true.
pub fn apply_otsu(pixels: &[u8], invert: bool) -> (Vec<u8>, f64) {
    let (t, variance) = otsu_threshold(pixels);
    let binarized = pixels
        .iter()
        .map(|&p| if (p >= t) ^ invert { 255 } else { 0 })
        .collect();
    (binarized, variance)
}

/// Measure how bimodal a grayscale intensity histogram is.
///
/// Uses the bimodality coefficient:
/// ```text
/// BC = (γ₁² + 1) / (γ₂ + 3·(n−1)²/((n−2)(n−3)))
/// ```
/// BC > 0.555 -> bimodal (Otsu reliable). BC ≤ 0.555 -> unimodal (prefer Sauvola).
/// Returns `0.0` for n < 4 or zero-variance images.
pub fn bimodality_coefficient(pixels: &[u8]) -> f32 {
    let n = pixels.len();
    if n < 4 {
        return 0.0;
    }
    let nf = n as f64;

    let mean = pixels.iter().map(|&p| p as f64).sum::<f64>() / nf;
    let mut m2 = 0.0f64;
    let mut m3 = 0.0f64;
    let mut m4 = 0.0f64;
    for &p in pixels {
        let d = p as f64 - mean;
        let d2 = d * d;
        m2 += d2;
        m3 += d2 * d;
        m4 += d2 * d2;
    }
    m2 /= nf;
    m3 /= nf;
    m4 /= nf;

    if m2 < 1e-10 {
        return 0.0;
    }

    let skewness = m3 / m2.powf(1.5);
    let excess_kurtosis = m4 / (m2 * m2) - 3.0;
    let correction = 3.0 * (nf - 1.0) * (nf - 1.0) / ((nf - 2.0) * (nf - 3.0));
    ((skewness * skewness + 1.0) / (excess_kurtosis + correction)) as f32
}

/// Score how "good" a binarized image looks.
///
/// Blends FDR tent function (weight 0.6) with Otsu inter-class variance
/// signal (weight 0.4, when `Some`). FDR tent peaks at 0.25, collapses to
/// zero for near-blank (< 2 %) or near-solid (> 92 %) results.
///
/// **Quality bands**: >= 0.60 = good/w | 0.30–0.59 = sus ⚠ | < 0.30 = bad/l
pub fn pipeline_quality_score(pixels: &[u8], otsu_variance: Option<f64>) -> f32 {
    let total = pixels.len() as f32;
    if total == 0.0 {
        return 0.0;
    }
    let fg = pixels.iter().filter(|&&p| p == 255).count() as f32;
    let fdr = fg / total;

    let fdr_score = if !(0.02..=0.92).contains(&fdr) {
        0.0f32
    } else {
        let left_half = 0.25f32 - 0.02;
        let right_half = 0.92f32 - 0.25;
        if fdr <= 0.25 {
            (fdr - 0.02) / left_half
        } else {
            (0.92 - fdr) / right_half
        }
        .max(0.0)
    };

    match otsu_variance {
        Some(var) => 0.6 * fdr_score + 0.4 * (var as f32 / 5000.0).min(1.0),
        None => fdr_score,
    }
}
