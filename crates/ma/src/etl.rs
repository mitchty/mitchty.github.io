//! # Supported formats
//!
//! | Format   | Datasets        | Chars                  | Image    | Bpp |
//! |----------|-----------------|------------------------|----------|-----|
//! | M-type   | ETL1, ETL6, ETL7| Katakana, Hiragana,... | 64x63    | 4   |
//! | 8B-type  | ETL8B, ETL9B    | Hiragana + Kanji       | 64x63    | 1   |
//! | 8G-type  | ETL8G, ETL9G    | Hiragana + Kanji       | 128x127  | 4   |
//!
//! Each parser yields `EtlRecord` values with the decoded 8-bit grayscale
//! pixels at their original resolution.  Use `resize_to_28x28` to downsample
//! before writing the output npz files.
//!
//! # References abused
//! - Official format docs: <https://etlcdb.db.aist.go.jp>
//! - Python struct layouts reverse-engineered from:
//!   - ETL8G: `'>2H8sI4B4H2B30x8128s11x'` (8199 bytes)
//!   - ETL8B: 512 bytes per record
//!   - M-type: 2052 bytes per record

use std::{collections::HashMap, fs, io};

/// Which binary format a file uses, only implemented a subset of the ETL
/// dataset before finding out it wasn't too useful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EtlFormat {
    /// ETL1, ETL6, ETL7: 2052 bytes/record, 64x63 4bpp, JIS X 0201
    M,
    /// ETL8B, ETL9B: 512 bytes/record, 64x63 1bpp, JIS X 0208
    B8,
    /// ETL8G, ETL9G: 8199 bytes/record, 128x127 4bpp, JIS X 0208
    G8,
}

impl EtlFormat {
    /// Parse a user-supplied format name, lowercased just in case... i'm punny
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "etl1" | "etl6" | "etl7" | "m" | "mtype" => Some(Self::M),
            "etl8b" | "etl9b" | "8b" | "b8" => Some(Self::B8),
            "etl8g" | "etl9g" | "8g" | "g8" => Some(Self::G8),
            _ => None,
        }
    }

    pub fn record_bytes(self) -> usize {
        match self {
            Self::M => 2052,
            Self::B8 => 512,
            Self::G8 => 8199,
        }
    }

    // TODO: iff I ever decide to resurrect this stuff this might be useful
    #[allow(dead_code)]
    pub fn image_width(self) -> u32 {
        match self {
            Self::M | Self::B8 => 64,
            Self::G8 => 128,
        }
    }

    #[allow(dead_code)]
    pub fn image_height(self) -> u32 {
        match self {
            Self::M | Self::B8 => 63,
            Self::G8 => 127,
        }
    }
}

/// A single decoded character record from any ETL file.
#[derive(Debug, Clone)]
pub struct EtlRecord {
    /// The Unicode character this record represents, if the code was
    /// recognised, which it probably won't be this dataset is noisy af
    pub character: Option<char>,
    /// Raw character code as stored in the file JIS X 0201 or JIS X 0208 apparently
    #[allow(dead_code)]
    pub raw_code: u16,
    /// Decoded grayscale pixels, `width x height` bytes, row-major, 0–255.
    pub pixels: Vec<u8>,
    /// Pixel grid width 64 or 128
    pub width: u32,
    /// Pixel grid height 63 or 127
    pub height: u32,
}

/// Read all records from a single etl file.
///
/// Silently skips truncated trailing bytes as some etl files appear to have a
/// short remainder at the end of the last block.
pub fn read_etl_file(path: &str, format: EtlFormat) -> io::Result<Vec<EtlRecord>> {
    let bytes = fs::read(path)?;
    let record_size = format.record_bytes();
    let n_records = bytes.len() / record_size;

    tracing::info!(
        path,
        format = ?format,
        record_size,
        n_records,
        "reading ETL file"
    );

    let mut records = Vec::with_capacity(n_records);
    for i in 0..n_records {
        let chunk = &bytes[i * record_size..(i + 1) * record_size];
        let rec = match format {
            EtlFormat::M => parse_mtype(chunk),
            EtlFormat::B8 => parse_8b(chunk),
            EtlFormat::G8 => parse_8g(chunk),
        };
        records.push(rec);
    }
    Ok(records)
}

/// Read every record from all ETL files in a directory, in sorted filename
/// order to try to be deterministic about things
pub fn read_etl_dir(dir: &str, format: EtlFormat) -> io::Result<Vec<EtlRecord>> {
    let mut paths: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();

    let mut all = Vec::new();
    for path in paths {
        let path_str = path.to_string_lossy();
        match read_etl_file(&path_str, format) {
            Ok(mut recs) => all.append(&mut recs),
            Err(e) => tracing::warn!("skipping {path_str}: {e}"),
        }
    }
    Ok(all)
}

// M-type ETL1, ETL6, ETL7 are 2052 bytes, 64x63 4bpp, JIS X 0201
//
// Byte layout BE:
//   0- 1  T56 character code           u16
//   2- 3  serial data number           u16
//   4- 5  JIS X 0201 character code    u16  == label
//   6- 7  EBCDIC character code        u16
//   8      quality grade               u8
//   9      complexity                  u8
//  10-11  sample number                u16
//  12-17  (pad / scanner info)         6 bytes
//  18-2033 image data 64x63 at 4bpp    2016 bytes  (64x63x4/8 = 2016)
//  2034-2051 (pad)                     18 bytes
fn parse_mtype(record: &[u8]) -> EtlRecord {
    assert_eq!(record.len(), 2052, "M-type record must be 2052 bytes");

    let jis_code = u16::from_be_bytes([record[4], record[5]]);

    // Image starts at byte 18, 64x63 4bpp packed high nibble first
    let img_bytes = &record[18..18 + 2016];
    let pixels = unpack_4bpp_to_8bit(img_bytes, 64 * 63);

    EtlRecord {
        character: jis0201_to_char(jis_code),
        raw_code: jis_code,
        pixels,
        width: 64,
        height: 63,
    }
}

// 8B-type ETL8B, ETL9B are 512 bytes, 64x63 1bpp, JIS X 0208
//
// Byte layout BE:
//   0- 1  serial number                u16
//   2- 3  JIS X 0208 character code    u16  == label
//   4      etype                       u8
//   5      eflag                       u8
//   6-509  image data 64x63 1bpp       504 bytes  (64x63/8 = 504)
//  510-511 (pad)                       2 bytes
fn parse_8b(record: &[u8]) -> EtlRecord {
    assert_eq!(record.len(), 512, "8B-type record must be 512 bytes");

    let jis_code = u16::from_be_bytes([record[2], record[3]]);

    let img_bytes = &record[6..6 + 504];
    let pixels = unpack_1bpp_to_8bit(img_bytes, 64 * 63);

    EtlRecord {
        character: jis0208_to_char(jis_code),
        raw_code: jis_code,
        pixels,
        width: 64,
        height: 63,
    }
}

// 8G-type ETL8G, ETL9G are 8199 bytes, 128x127 4bpp, JIS X 0208
//
// Python struct: '>2H8sI4B4H2B30x8128s11x'  (total = 8199 bytes)
//   0- 1  serial                       2xu16 = 4 bytes ... but struct says 2H = 4 bytes
//   Actually: 2H = 4, 8s = 8, I = 4, 4B = 4, 4H = 8, 2B = 2, 30x = 30, 8128s = 8128, 11x = 11
//   Total: 4+8+4+4+8+2+30+8128+11 = 8199
//
//   offset 0:  2xu16 BE  serial_number, sheet_number
//   offset 4:  8 bytes   JIS code string e.g. b'.HIRA..' or b'\xb4\xa2..'
//   offset 12: u32 BE    info
//   offset 16: 4xu8      quality flags
//   offset 20: 4xu16 BE  measurements
//   offset 28: 2xu8      flags
//   offset 30: 30 bytes  padding
//   offset 60: 8128 bytes image 128x127 4bpp  (128x127x4/8 = 8128)
//   offset 8188: 11 bytes padding
//
// Thank god for python examples on the web for this...
fn parse_8g(record: &[u8]) -> EtlRecord {
    assert_eq!(record.len(), 8199, "8G-type record must be 8199 bytes");

    // The 8-byte JIS field starting at offset 4 holds a JIS X 0208 2-byte code
    // in the first two bytes; the rest is annotation text like '.HIRA'

    // When the first byte >= 0xa4 and <= 0xf4 it's an EUC-JP encoded
    // JIS X 0208 row/col. Convert EUC to JIS by subtracting 0x80 from each byte.
    let jis_raw = &record[4..12];
    let jis_code = euc_to_jis(jis_raw[0], jis_raw[1]);

    let img_bytes = &record[60..60 + 8128];
    let pixels = unpack_4bpp_to_8bit(img_bytes, 128 * 127);

    EtlRecord {
        character: jis0208_to_char(jis_code),
        raw_code: jis_code,
        pixels,
        width: 128,
        height: 127,
    }
}

/// Unpack `n` c type nibbles aka 4-bit values from `src` into a single 8bpp
/// grayscale.
/// Each nibble is scaled `v * 17` so that 0xF = 255, 0x0 = 0 to help the CNN.
fn unpack_4bpp_to_8bit(src: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    for byte in src {
        let hi = (byte >> 4) & 0x0f;
        let lo = byte & 0x0f;
        out.push(hi * 17);
        if out.len() < n {
            out.push(lo * 17);
        }
    }
    out.truncate(n);
    out
}

/// Unpack `n` 1-bit values from `src` into 8-bit (0 or 255), MSB first.
fn unpack_1bpp_to_8bit(src: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    'outer: for byte in src {
        for bit in (0..8).rev() {
            out.push(if (byte >> bit) & 1 == 1 { 255 } else { 0 });
            if out.len() == n {
                break 'outer;
            }
        }
    }
    out
}

/// Convert a JIS X 0201 code to corresponding Unicode character.
///
/// JIS X 0201 range 0x20–0x7e = ASCII printable.
/// Range 0xa1–0xdf = half-width katakana mapped to U+FF61–U+FF9F.
// TODO: others? JIS is crazy
pub fn jis0201_to_char(code: u16) -> Option<char> {
    let b = code as u8;
    match b {
        0x20..=0x7e => char::from_u32(b as u32),
        0xa1..=0xdf => char::from_u32(0xff61 + (b - 0xa1) as u32),
        _ => None,
    }
}

/// Convert a JIS X 0208 2-byte code to corresponding Unicode character.
///
/// The input is the raw 2-byte code as stored in ETL8B/9B records
/// big-endian: row byte, column byte, each in the range 0x21–0x7e
///
/// Uses EUC-JP decoding via `encoding_rs` for this bit.
pub fn jis0208_to_char(code: u16) -> Option<char> {
    let row = (code >> 8) as u8;
    let col = (code & 0xff) as u8;

    if !(0x21..=0x7e).contains(&row) || !(0x21..=0x7e).contains(&col) {
        return None;
    }

    let euc = [row | 0x80, col | 0x80];
    let (decoded, _, had_errors) = encoding_rs::EUC_JP.decode(&euc);
    if had_errors {
        return None;
    }
    decoded.chars().next()
}

/// Map EUC-JP 2-byte sequence to a JIS X 0208 char code.
/// EUC stores bytes as `jis_byte | 0x80`, so we mask off the high bit.
// TODO: JIS is not my favorite encoding scheme, is there a library I can use
// for this?
fn euc_to_jis(b1: u8, b2: u8) -> u16 {
    ((b1 & 0x7f) as u16) << 8 | (b2 & 0x7f) as u16
}

/// Resize a `src_w x src_h` 8bpp grayscale image to 28x28 to match mnist layout.
///
/// Uses bilinear resampling rather than Lanczos3. Lanczos3 produced halos
/// around the strokes which just gave the training CNN a stroke of its own.
///
/// Returns a 784-byte row major array
pub fn resize_to_28(pixels: &[u8], src_w: u32, src_h: u32) -> [u8; 784] {
    use image::{GrayImage, imageops};

    let img = GrayImage::from_raw(src_w, src_h, pixels.to_vec())
        .expect("pixel buffer must match src_w x src_h");

    let resized = imageops::resize(&img, 28, 28, imageops::FilterType::Triangle);

    let mut out = [0u8; 784];
    out.copy_from_slice(resized.as_raw());
    out
}

/// Write a single npy v1 array of `dtype = |u1` (u8) into a `.npz` file.
///
/// `shape` is e.g. `[N, 28, 28]` for images or `[N]` for labels.
/// The array name inside the zip is always `arr_0.npy` to match the convention
/// used by `NpzDataset::read_npz_first_entry` in `data.rs`.
/// Writes a numpy v1 npz file containing a single array `arr_0.npy`.
///
/// `dtype` must be a valid (subset of a) numpy dtype string aka `"|u1"` uint8 or `"<u2"` uint16 LE.
pub fn write_npz(path: &str, data: &[u8], shape: &[usize], dtype: &str) -> io::Result<()> {
    use std::io::Write;

    let shape_str = shape
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    // NPY v1 header dict: dtype, fortran_order, shape
    let header_dict =
        format!("{{'descr': '{dtype}', 'fortran_order': False, 'shape': ({shape_str},), }}");
    // Header must be padded to a multiple of 64 bytes including the 10-byte prefix to be compatible
    let prefix_len = 10usize;
    let raw_len = header_dict.len() + 1; // +1 for '\n' terminator
    // div_ceil gives the number of 64-byte blocks; multiply back to get the
    // total padded byte count, then subtract to find how many spaces to add.
    let padded_total = (prefix_len + raw_len).div_ceil(64) * 64;
    let pad_len = padded_total - prefix_len - raw_len;

    let header_len = (raw_len + pad_len) as u16;

    let mut npy: Vec<u8> = Vec::new();
    npy.extend_from_slice(b"\x93NUMPY"); // magic
    npy.push(1); // major version
    npy.push(0); // minor version
    npy.extend_from_slice(&header_len.to_le_bytes());
    npy.extend_from_slice(header_dict.as_bytes());
    npy.extend(std::iter::repeat_n(b' ', pad_len));
    npy.push(b'\n');
    npy.extend_from_slice(data);

    let file = fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("arr_0.npy", options)?;
    zip.write_all(&npy)?;
    zip.finish()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// High-level conversion
// ---------------------------------------------------------------------------

/// Resolved output paths produced by [`output_paths`].
pub struct OutputPaths {
    pub imgs: String,
    pub labels: String,
    pub test_imgs: Option<String>,
    pub test_labels: Option<String>,
}

/// Derive all output file paths from a single output directory and a train
/// fraction representing percent of records for validation versus train.
///
/// | train_fraction | files written                                        |
/// |----------------|------------------------------------------------------|
/// | 0.0            | `{dir}/imgs.npz`, `{dir}/labels.npz`                 |
/// | 0.0..1.0       | `{dir}/train-imgs.npz`, `{dir}/train-labels.npz`,    |
/// |                | `{dir}/test-imgs.npz`,  `{dir}/test-labels.npz`      |
// TODO: I am not sure this is too great of an approach but WET vs DRY.
pub fn output_paths(out_dir: &str, train_fraction: f64) -> OutputPaths {
    let p = |name: &str| format!("{out_dir}/{name}");
    if train_fraction > 0.0 {
        OutputPaths {
            imgs: p("train-imgs.npz"),
            labels: p("train-labels.npz"),
            test_imgs: Some(p("test-imgs.npz")),
            test_labels: Some(p("test-labels.npz")),
        }
    } else {
        OutputPaths {
            imgs: p("imgs.npz"),
            labels: p("labels.npz"),
            test_imgs: None,
            test_labels: None,
        }
    }
}

/// Convert ETL records to npz files using `OutputPaths`
///
/// `label_map` maps Unicode char to u32 class index.  Records if a character
/// is not in the map, or could not be decoded, are simply silently dropped.
///
/// Labels are written as `|u1` (1 byte) when num_classes ≤ 255, and as `<u2`
/// (2-byte little-endian u16) when num_classes > 255. `NpzDataset` detects the
/// dtype from the NPY header and reads both automatically so this should still
/// be usable in python lande.
///
/// When `paths.test_imgs` / `paths.test_labels` are `Some`, the data is split
/// at `train_n = round(total * train_fraction)` items.
pub fn convert_to_npz(
    records: &[EtlRecord],
    paths: &OutputPaths,
    label_map: &HashMap<char, u32>,
    train_fraction: f64,
) -> io::Result<()> {
    let mut images: Vec<u8> = Vec::new();
    let mut labels_u32: Vec<u32> = Vec::new();

    for rec in records {
        let Some(ch) = rec.character else { continue };
        let Some(&label) = label_map.get(&ch) else {
            continue;
        };
        let resized = resize_to_28(&rec.pixels, rec.width, rec.height);
        images.extend_from_slice(&resized);
        labels_u32.push(label);
    }

    let num_classes = label_map.len();
    let use_u16 = num_classes > 255;

    // Serialize labels to bytes in the appropriate dtype for the file.
    let (label_bytes, label_dtype): (Vec<u8>, &str) = if use_u16 {
        let bytes = labels_u32
            .iter()
            .flat_map(|&l| (l as u16).to_le_bytes())
            .collect();
        (bytes, "<u2")
    } else {
        let bytes = labels_u32.iter().map(|&l| l as u8).collect();
        (bytes, "|u1")
    };

    let n = labels_u32.len();
    if n == 0 {
        tracing::warn!("no matching records found nothing written");
        return Ok(());
    }

    // Derive the output dir from paths.imgs file it's always "{dir}/FILE" I think...
    let out_dir = std::path::Path::new(&paths.imgs)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".");
    write_classmap(out_dir, label_map)?;
    write_stats(out_dir, &images)?;

    tracing::info!(n, classes = num_classes, label_dtype, "writing npz files");

    // Bytes per label in its serialised form
    let label_stride = if use_u16 { 2 } else { 1 };

    match (&paths.test_imgs, &paths.test_labels) {
        (Some(test_imgs), Some(test_labels)) => {
            let train_n = ((n as f64 * train_fraction).round() as usize).clamp(1, n - 1);

            write_npz(
                &paths.imgs,
                &images[..train_n * 784],
                &[train_n, 28, 28],
                "|u1",
            )?;
            write_npz(
                &paths.labels,
                &label_bytes[..train_n * label_stride],
                &[train_n],
                label_dtype,
            )?;
            write_npz(
                test_imgs,
                &images[train_n * 784..],
                &[n - train_n, 28, 28],
                "|u1",
            )?;
            write_npz(
                test_labels,
                &label_bytes[train_n * label_stride..],
                &[n - train_n],
                label_dtype,
            )?;

            tracing::info!(
                train = train_n,
                test  = n - train_n,
                train_imgs   = %paths.imgs,
                train_labels = %paths.labels,
                test_imgs    = %test_imgs,
                test_labels  = %test_labels,
                "split written"
            );
        }
        _ => {
            write_npz(&paths.imgs, &images, &[n, 28, 28], "|u1")?;
            write_npz(&paths.labels, &label_bytes, &[n], label_dtype)?;
            tracing::info!(n, imgs = %paths.imgs, labels = %paths.labels, "written");
        }
    }

    Ok(())
}

/// Write `{out_dir}/classmap.json` a glorified JSON array of single-char strings in
/// label-index order, e.g. `["あ","い","う",…,"一","二",…]`.
///
/// The array index equals the class label used in the npz files, so
/// `classmap[i]` is the Unicode character for class `i`.  This file is
/// consumed by `ma train --classmap` to embed the mapping in `config.json`,
/// from which the inference engine loads all the indices at runtime.
// TODO: This was half thought out. Could do much more here.
pub fn write_classmap(out_dir: &str, label_map: &HashMap<char, u32>) -> io::Result<()> {
    let n = label_map
        .values()
        .copied()
        .map(|v| v as usize + 1)
        .max()
        .unwrap_or(0);
    let mut chars = vec!['\0'; n];
    for (&ch, &idx) in label_map {
        chars[idx as usize] = ch;
    }

    // Serialize as a JSON array of UTF-8 single-char strings directly as utf8
    let json = {
        let entries: Vec<String> = chars.iter().map(|c| format!("\"{}\"", c)).collect();
        format!("[{}]\n", entries.join(","))
    };

    let path = format!("{out_dir}/classmap.json");
    fs::write(&path, json.as_bytes())?;
    tracing::info!(path, classes = n, "classmap written");
    Ok(())
}

/// Write `{out_dir}/stats.json` file with the per-pixel mean and std computed
/// from the entire image corpus. Note any split is not taken into account here.
/// Not sure if that was right or not I abandoned this dataset about this time.
///
/// The values are computed over all images at once, treating each of the
/// 784 pixels as an independent sample, yielding accurate population stats
/// for use as normalization constants at training and inference time.
///
/// `images` is the flat `u8` buffer: `N x 784` bytes, values in `[0, 255]`.
pub fn write_stats(out_dir: &str, images: &[u8]) -> io::Result<()> {
    let n = images.len() as f64;
    if n == 0.0 {
        return Ok(());
    }

    // Compute mean and std for the pixel data in a single pass. Values
    // scaled to [0,1] to make the CNN training happier.
    let sum: f64 = images.iter().map(|&v| v as f64 / 255.0).sum();
    let sum_sq: f64 = images.iter().map(|&v| (v as f64 / 255.0).powi(2)).sum();
    let mean = sum / n;
    let std = ((sum_sq / n) - mean * mean).sqrt();

    // TODO: Yeah its lazy and should be in serde. This is half finished code at
    // best. The data contained is full of noise that made it not useful for
    // CNN's
    let json = format!(
        "{{\n  \"norm_mean\": {:.6},\n  \"norm_std\": {:.6},\n  \"n_images\": {}\n}}\n",
        mean,
        std,
        n as u64 / 784,
    );

    let path = format!("{out_dir}/stats.json");
    fs::write(&path, json.as_bytes())?;
    tracing::info!(
        path,
        mean = format!("{mean:.4}"),
        std = format!("{std:.4}"),
        "stats written"
    );
    Ok(())
}

/// Build a map from a slice of Unicode chars: char to sequential u32 index,
/// this was intended to join the ETL datasets together but then I looked at the
/// data and noped out of further work here entirely.
pub fn build_label_map(chars: &[char]) -> HashMap<char, u32> {
    chars
        .iter()
        .copied()
        .enumerate()
        .map(|(i, c)| (c, i as u32))
        .collect()
}

/// Build a map that includes every decodable character in `records`.
pub fn label_map_from_records(records: &[EtlRecord]) -> HashMap<char, u32> {
    let mut seen: Vec<char> = Vec::new();
    for rec in records {
        if let Some(ch) = rec.character
            && !seen.contains(&ch)
        {
            seen.push(ch);
        }
    }
    seen.sort_unstable();
    build_label_map(&seen)
}
