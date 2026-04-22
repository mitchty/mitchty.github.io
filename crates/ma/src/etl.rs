//! # Supported formats
//!
//! | Format   | Datasets        | Chars                  | Image    | Bpp |
//! |----------|-----------------|------------------------|----------|-----|
//! | M-type   | ETL1, ETL6, ETL7| Katakana, Hiragana,... | 64x63    | 4   |
//! | B8-type  | ETL8B           | Hiragana + Kanji       | 64x63    | 1   |
//! | B9-type  | ETL9B           | Hiragana + Kanji       | 64x63    | 1   |
//! | G8-type  | ETL8G           | Hiragana + Kanji       | 128x127  | 4   |
//! | G9-type  | ETL9G           | Hiragana + Kanji       | 128x127  | 4   |
//! | C-type   | ETL3, ETL4, ETL5| Katakana / Hiragana    | 72x76    | 4   |
//! | K-type   | ETL2            | CO-59 (2184 chars)     | 60x60    | 6   |
//!
//! Each parser yields `EtlRecord` values with the decoded 8-bit grayscale
//! pixels at their original resolution. Use `resize_to_28` / `resize_to_64`
//! to downsample before writing the output npz files.
//!
//! # C-type and K-type: 6-bit character addressing
//!
//! ETL2–5 store records as packed 6-bit character streams. Conveniently, the
//! key field start positions happen to land on exact byte boundaries:
//!
//! C-type (2952 bytes = 3936 x 6-bit chars):
//!   - JIS label: top 8 bits of chars 12–17 (0-indexed) = bits 72–79 = byte 9
//!   - Image:     chars 288–3935 = bits 1728+ = byte 216, 4bpp, 72x76
//!
//! K-type (2745 bytes = 3660 x 6-bit chars):
//!   - CO-59 code: chars 28–29 = bits 168–179 = byte 21 (upper 6) + byte 22 (next 6)
//!     upper 6 bits = column index, lower 6 bits = row index in CO-59 grid
//!   - Image:     chars 60–3659 = bits 360+ = byte 45, 6bpp, 60x60
//!   - CO-59 -> Unicode mapping implemented via `co59_to_char()` using the
//!     full 2304-entry table from AIST's `euc_co59.dat`.
//!
//! # Official format references (authoritative byte layouts)
//! - M-type:  <https://etlcdb.db.aist.go.jp/etlcdb/etln/form_m.htm>
//! - B-type:  <https://etlcdb.db.aist.go.jp/etlcdb/etln/form_e8b.htm>
//!   <https://etlcdb.db.aist.go.jp/etlcdb/etln/form_e9b.htm>
//! - G-type:  <https://etlcdb.db.aist.go.jp/etlcdb/etln/form_e8g.htm>
//!   <https://etlcdb.db.aist.go.jp/etlcdb/etln/form_e9g.htm>
//! - C-type:  <https://etlcdb.db.aist.go.jp/etlcdb/etln/form_c.htm>
//! - K-type:  <https://etlcdb.db.aist.go.jp/etlcdb/etln/form_k.htm>

mod co59;

use std::{
    collections::HashMap,
    fs,
    io::{self, BufReader, Read, Seek, SeekFrom},
    time::Duration,
};

use indicatif::{HumanBytes, HumanCount, ProgressBar, ProgressStyle};

// ---------------------------------------------------------------------------
// Character filtering helpers (shared by convert_etlcdb and kanjivg paths)
// ---------------------------------------------------------------------------

/// Why a character was excluded from the output dataset.
#[derive(Debug)]
pub(crate) enum FilterReason {
    /// Explicitly listed via `--filter`.
    ExplicitChar,
    /// Unicode name contained a `--filter-name` substring.
    /// Fields are used via the `Debug` impl for log output only.
    #[allow(dead_code)]
    NameSubstring { pattern: String, name: String },
}

/// Test whether `ch` should be excluded from the output.
///
/// Returns `Some(reason)` to drop the character, `None` to keep it.
/// `filter_name_lc` must already be lowercased by the caller.
pub(crate) fn filter_reason(
    ch: char,
    filter_chars: &[char],
    filter_name_lc: &[String],
) -> Option<FilterReason> {
    if filter_chars.contains(&ch) {
        return Some(FilterReason::ExplicitChar);
    }
    if !filter_name_lc.is_empty() {
        let name = unicode_names2::name(ch)
            .map(|n| n.to_string())
            .unwrap_or_default();
        let name_lc = name.to_lowercase();
        if let Some(pattern) = filter_name_lc
            .iter()
            .find(|sub| name_lc.contains(sub.as_str()))
        {
            return Some(FilterReason::NameSubstring {
                pattern: pattern.clone(),
                name,
            });
        }
    }
    None
}

/// Which binary format a file uses.
///
/// ETL8B and ETL9B share the same header layout and image data but have
/// different total record sizes (512 vs 576 bytes). ETL8G and ETL9G differ
/// in the amount of padding before the image (offset 60 vs 64).
///
/// C-type (ETL3/4/5) and K-type (ETL2) use 6-bit character addressing
/// (1 character = 6 bits) rather than standard byte addressing; the key
/// fields happen to fall on byte boundaries so parsing is fully implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EtlFormat {
    /// ETL1, ETL6, ETL7: 2052 bytes/record, 64x63 4bpp, JIS X 0201
    M,
    /// ETL8B: 512 bytes/record, 64x63 1bpp, JIS X 0208; first record is dummy
    B8,
    /// ETL9B: 576 bytes/record, 64x63 1bpp, JIS X 0208; first record is dummy
    B9,
    /// ETL8G: 8199 bytes/record, 128x127 4bpp, JIS X 0208; image at byte 61 (offset 60)
    G8,
    /// ETL9G: 8199 bytes/record, 128x127 4bpp, JIS X 0208; image at byte 65 (offset 64)
    G9,
    /// ETL3, ETL4, ETL5: 2952 bytes/record (3936 6-bit chars), 72x76 4bpp, JIS X 0201
    C,
    /// ETL2: 2745 bytes/record (3660 6-bit chars), 60x60 6bpp, CO-59 code
    K,
}

impl EtlFormat {
    /// Parse a user-supplied format name.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "etl1" | "etl6" | "etl7" | "m" | "mtype" => Some(Self::M),
            "etl8b" | "8b" | "b8" => Some(Self::B8),
            "etl9b" | "9b" | "b9" => Some(Self::B9),
            "etl8g" | "8g" | "g8" => Some(Self::G8),
            "etl9g" | "9g" | "g9" => Some(Self::G9),
            "etl3" | "etl4" | "etl5" | "c" | "ctype" => Some(Self::C),
            "etl2" | "k" | "ktype" => Some(Self::K),
            _ => None,
        }
    }

    pub fn record_bytes(self) -> usize {
        match self {
            Self::M => 2052,
            Self::B8 => 512,
            Self::B9 => 576,
            Self::G8 | Self::G9 => 8199,
            // 3936 6-bit chars = 2952 bytes
            Self::C => 2952,
            // 3660 6-bit chars = 2745 bytes
            Self::K => 2745,
        }
    }

    /// Whether the first record in each file is a dummy to be skipped.
    pub fn has_dummy_first_record(self) -> bool {
        matches!(self, Self::B8 | Self::B9)
    }

    /// Native pixel width of images in this format.
    #[allow(dead_code)]
    pub fn image_width(self) -> u32 {
        match self {
            Self::M | Self::B8 | Self::B9 => 64,
            Self::G8 | Self::G9 => 128,
            Self::C => 72,
            Self::K => 60,
        }
    }

    /// Native pixel height of images in this format.
    #[allow(dead_code)]
    pub fn image_height(self) -> u32 {
        match self {
            Self::M | Self::B8 | Self::B9 => 63,
            Self::G8 | Self::G9 => 127,
            Self::C => 76,
            Self::K => 60,
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
    /// Source file path this record was loaded from (for debugging/traceability)
    pub source_file: Option<String>,
}

/// A batch of records all sharing the same native image dimensions.
///
/// When converting the entire ETLcdb at once, M/B-type (64x63) and G-type
/// (128x127) records cannot be stored in a single npz array because npz arrays
/// are uniform in shape. `EtlBatch` groups records by native resolution so
/// each group can be written to its own npz file.
#[derive(Debug)]
pub struct EtlBatch {
    /// Short tag for this batch used as a filename prefix, e.g. `"m"`, `"b"`, `"g"`.
    pub tag: &'static str,
    /// All decoded records in this batch - all share the same `width`/`height`.
    pub records: Vec<EtlRecord>,
}

/// Discover and read all supported ETL datasets under `root`.
///
/// `root` is the parent directory that contains sub-directories named
/// `ETL1`, `ETL2`, …, `ETL9G`. Each recognised sub-directory is read with
/// its correct [`EtlFormat`]. Unrecognised sub-directories and sub-directories
/// that are absent on disk are silently skipped (with a log warning so the
/// caller knows something was skipped).
///
/// `exclude` is a list of dataset names (e.g., "ETL2", "ETL8G") to skip entirely.
/// This is useful for excluding problematic datasets at runtime.
///
/// Records are returned in **five batches** grouped by native image
/// dimensions, because a single npz array must have a uniform shape:
///
/// | batch tag | format family | native size | datasets           |
/// |-----------|---------------|-------------|--------------------|
/// | `"m"`     | M-type        | 64 x 63     | ETL1, ETL6, ETL7   |
/// | `"k"`     | K-type        | 60 x 60     | ETL2               |
/// | `"c"`     | C-type        | 72 x 76     | ETL3, ETL4, ETL5   |
/// | `"b"`     | B-type        | 64 x 63     | ETL8B, ETL9B       |
/// | `"g"`     | G-type        | 128 x 127   | ETL8G, ETL9G       |
///
/// Batches with zero records are omitted from the returned `Vec`.
pub fn read_etlcdb_dir(root: &str, exclude: &[&str]) -> io::Result<Vec<EtlBatch>> {
    // (subdir_name, format, batch_tag)
    // Ordered ETL1 -> ETL9G so logs read in dataset-number order.
    // Batch tags group by native image dimensions:
    //   "m"  M-type  64x63   ETL1, ETL6, ETL7
    //   "b"  B-type  64x63   ETL8B, ETL9B
    //   "g"  G-type  128x127 ETL8G, ETL9G
    //   "c"  C-type  72x76   ETL3, ETL4, ETL5
    //   "k"  K-type  60x60   ETL2
    let known: &[(&str, EtlFormat, &str)] = &[
        ("ETL1", EtlFormat::M, "m"),
        ("ETL2", EtlFormat::K, "k"),
        ("ETL3", EtlFormat::C, "c"),
        ("ETL4", EtlFormat::C, "c"),
        ("ETL5", EtlFormat::C, "c"),
        ("ETL6", EtlFormat::M, "m"),
        ("ETL7", EtlFormat::M, "m"),
        ("ETL8B", EtlFormat::B8, "b"),
        ("ETL9B", EtlFormat::B9, "b"),
        ("ETL8G", EtlFormat::G8, "g"),
        ("ETL9G", EtlFormat::G9, "g"),
    ];

    // Accumulate into per-tag buckets.
    let mut m_records: Vec<EtlRecord> = Vec::new();
    let mut b_records: Vec<EtlRecord> = Vec::new();
    let mut g_records: Vec<EtlRecord> = Vec::new();
    let mut c_records: Vec<EtlRecord> = Vec::new();
    let mut k_records: Vec<EtlRecord> = Vec::new();

    for &(subdir, format, tag) in known {
        let dir_path = std::path::Path::new(root).join(subdir);
        if !dir_path.exists() {
            tracing::warn!(dir = subdir, "directory not found, skipping");
            continue;
        }
        let dir_str = dir_path.to_string_lossy();

        // Check if this specific dataset directory should be excluded
        // Also check if the full path matches (for absolute path exclusions)
        let is_dir_excluded = exclude.contains(&subdir) || exclude.contains(&dir_str.as_ref());
        if is_dir_excluded {
            tracing::info!(dir = subdir, "excluded, skipping entire directory");
            continue;
        }

        tracing::info!(dir = subdir, format = ?format, "reading ETL directory");
        match read_etl_dir(&dir_str, format, exclude) {
            Ok(mut recs) => {
                tracing::info!(dir = subdir, n = recs.len(), "records loaded");
                match tag {
                    "m" => m_records.append(&mut recs),
                    "b" => b_records.append(&mut recs),
                    "g" => g_records.append(&mut recs),
                    "c" => c_records.append(&mut recs),
                    "k" => k_records.append(&mut recs),
                    _ => unreachable!(),
                }
            }
            Err(e) => tracing::warn!(dir = subdir, error = %e, "failed to read, skipping"),
        }
    }

    let mut batches = Vec::new();
    if !m_records.is_empty() {
        batches.push(EtlBatch {
            tag: "m",
            records: m_records,
        });
    }
    if !b_records.is_empty() {
        batches.push(EtlBatch {
            tag: "b",
            records: b_records,
        });
    }
    if !g_records.is_empty() {
        batches.push(EtlBatch {
            tag: "g",
            records: g_records,
        });
    }
    if !c_records.is_empty() {
        batches.push(EtlBatch {
            tag: "c",
            records: c_records,
        });
    }
    if !k_records.is_empty() {
        let decoded = k_records.iter().filter(|r| r.character.is_some()).count();
        tracing::info!(
            n = k_records.len(),
            decoded,
            "K-type (ETL2) records loaded - CO-59 -> Unicode mapping applied"
        );
        batches.push(EtlBatch {
            tag: "k",
            records: k_records,
        });
    }
    Ok(batches)
}

/// Read all records from a single ETL file.
///
/// Reads one record at a time via a `BufReader` so the raw file bytes are
/// never all in memory simultaneously; only the current record-sized chunk
/// and the decoded `Vec<EtlRecord>` are live at the same time.
///
/// Silently stops when fewer than `record_size` bytes remain (truncated tail).
///
/// For formats with a dummy first record (ETL8B, ETL9B) record index 0 is
/// skipped automatically so callers always receive real character data.
pub fn read_etl_file(path: &str, format: EtlFormat) -> io::Result<Vec<EtlRecord>> {
    let record_size = format.record_bytes();

    // Determine total record count from file size so we can pre-allocate and
    // know how many to skip, without reading the whole file into RAM.
    let file_len = fs::metadata(path)?.len() as usize;
    let n_records = file_len / record_size;

    // ETL8B and ETL9B files begin with one dummy record that carries no
    // character data. Skip it so the caller only ever sees real samples.
    let first = if format.has_dummy_first_record() {
        1
    } else {
        0
    };

    tracing::info!(
        path,
        format = ?format,
        record_size,
        n_records,
        skipping_dummy = first,
        "reading ETL file"
    );

    // Source file path for debugging/traceability
    let source_file = Some(path.to_string());

    // ETL4 (C-type) and ETL7 (M-type) store hiragana images but label them
    // with JIS X 0201 katakana codes - the encoding has no hiragana range.
    // Detect by checking whether any path component is exactly "ETL4"/"ETL7"
    // so that /data/ETL4/ETL4_1 matches but /data/ETL4X/foo does not.
    let path_obj = std::path::Path::new(path);
    let is_etl4 = path_obj.components().any(|c| c.as_os_str() == "ETL4");
    let is_etl7 = path_obj.components().any(|c| c.as_os_str() == "ETL7");

    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);

    // Skip the dummy first record(s) by seeking past them.
    if first > 0 {
        reader.seek(SeekFrom::Start((first * record_size) as u64))?;
    }

    let mut chunk = vec![0u8; record_size];
    let mut records = Vec::with_capacity(n_records.saturating_sub(first));

    for _ in first..n_records {
        match reader.read_exact(&mut chunk) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let rec = match format {
            EtlFormat::M => parse_mtype(&chunk, source_file.clone(), is_etl7),
            EtlFormat::B8 | EtlFormat::B9 => parse_8b(&chunk, source_file.clone()),
            EtlFormat::G8 => parse_8g(&chunk, 60, source_file.clone()),
            EtlFormat::G9 => parse_8g(&chunk, 64, source_file.clone()),
            EtlFormat::C => parse_ctype(&chunk, source_file.clone(), is_etl4),
            EtlFormat::K => parse_ktype(&chunk, source_file.clone()),
        };
        records.push(rec);
    }
    Ok(records)
}

/// Read every record from all ETL files in a directory, in sorted filename
/// order to try to be deterministic about things.
///
/// `exclude` is a list of patterns to skip. Each pattern can be:
/// - A full filename (e.g., "ETL2_1" or "ETL2_1.gz")
/// - A directory name (e.g., "ETL2") which excludes all files in that directory
pub fn read_etl_dir(dir: &str, format: EtlFormat, exclude: &[&str]) -> io::Result<Vec<EtlRecord>> {
    let record_size = format.record_bytes();

    let dir_path = std::path::Path::new(dir);
    let dir_name = dir_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    let mut paths: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            if !p.is_file() {
                return false;
            }

            let filename = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let full_path = p.to_string_lossy();

            // Check if this file or directory is excluded
            // Patterns can match against:
            // - filename only (e.g., "ETL2_1")
            // - directory name (e.g., "ETL2")
            // - full path (e.g., "/path/to/etlcdb/ETL2/ETL2_1")
            let is_excluded = exclude
                .iter()
                .any(|pat| filename == *pat || dir_name == *pat || full_path.contains(pat));

            if is_excluded {
                tracing::debug!(path = %p.display(), "excluded, skipping");
                return false;
            }

            // Reject any file whose byte length is not an exact multiple of the
            // record size. All legitimate ETL data files are densely packed
            // records with no header or trailer, so their size is always
            // `n_records x record_size` exactly. Text files like `ETL8INFO`
            // that share a directory with the data files are never exact
            // multiples and must be skipped; parsing them as records produces
            // spurious phantom characters from the garbage bytes at [2..4].
            match p.metadata() {
                Ok(m) => {
                    let len = m.len() as usize;
                    //                    if len % record_size != 0 {
                    if !len.is_multiple_of(record_size) {
                        tracing::debug!(
                            path = %p.display(),
                            len,
                            record_size,
                            "skipping: file size not a multiple of record size"
                        );
                        false
                    } else {
                        true
                    }
                }
                Err(_) => false,
            }
        })
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

// M-type ETL1, ETL6, ETL7 - 2052 bytes/record, 64x63 4bpp, JIS X 0201
//
// Official layout (1-indexed, from form_m.htm):
//   bytes  1- 2  Data Number                          u16
//   bytes  3- 4  Character Code (ASCII, 2 bytes)      2xu8
//   bytes  5- 6  Serial Sheet Number                  u16
//   byte   7     JIS X 0201 code                      u8   ← label
//   byte   8     EBCDIC code                          u8
//   byte   9     Quality of individual image (0–3)    u8
//   byte  10     Quality of group (0–2)               u8
//   byte  11     Male/Female code                     u8
//   byte  12     Age of writer                        u8
//   bytes 13-16  Serial Data Number                   u32
//   bytes 17-18  Industry Classification Code         u16
//   bytes 19-20  Occupation Classification Code       u16
//   bytes 21-22  Sheet Gathering Date                 u16
//   bytes 23-24  Scanning Date                        u16
//   byte  25     Sample Position Y                    u8
//   byte  26     Sample Position X                    u8
//   byte  27     Minimum Scanned Level                u8
//   byte  28     Maximum Scanned Level                u8
//   bytes 29-32  (undefined)                          4 bytes
//   bytes 33-2048 Image data 64x63 @ 4bpp             2016 bytes
//   bytes 2049-2052 (uncertain)                       4 bytes
//
// 0-indexed equivalents used below:
//   offset  6     -> JIS X 0201 byte
//   offset 32..48 -> image start (32 = byte 33 − 1)
// `hiragana` is set for ETL7, which stores hiragana images labelled with JIS
// X 0201 katakana codes. When true, the katakana range is remapped to the
// phonetically equivalent hiragana codepoints via `jis0201_kana_as_hiragana`.
fn parse_mtype(record: &[u8], source_file: Option<String>, hiragana: bool) -> EtlRecord {
    assert_eq!(record.len(), 2052, "M-type record must be 2052 bytes");

    // Single-byte JIS X 0201 code at offset 6 (byte 7 in 1-indexed spec).
    let jis_byte = record[6];

    // Image data starts at offset 32 (byte 33 in 1-indexed spec): 2016 bytes
    // of 64x63 pixels packed as 4bpp, high nibble first.
    let img_bytes = &record[32..32 + 2016];
    let pixels = unpack_4bpp_to_8bit(img_bytes, 64 * 63);

    let character = if hiragana {
        jis0201_kana_as_hiragana(jis_byte)
    } else {
        jis0201_to_char(jis_byte)
    };
    let src_stem = source_file
        .as_deref()
        .and_then(|p| {
            std::path::Path::new(p)
                .file_stem()?
                .to_str()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned());
    tracing::debug!(
        source = %src_stem,
        raw_code = format!("0x{jis_byte:02X}"),
        hiragana_mode = hiragana,
        char = character.map(|c| c.to_string()).unwrap_or_else(|| "none".to_owned()),
        codepoint = character.map(|c| format!("U+{:04X}", c as u32)).unwrap_or_else(|| "none".to_owned()),
        "loaded class record"
    );

    EtlRecord {
        character,
        raw_code: jis_byte as u16,
        pixels,
        width: 64,
        height: 63,
        source_file,
    }
}

// B-type ETL8B / ETL9B - 1bpp, 64x63, JIS X 0208
//
// Official layout (1-indexed, from form_e8b.htm / form_e9b.htm):
//   bytes  1- 2  Serial Sheet Number              u16
//   bytes  3- 4  JIS X 0208 code (2-byte binary)  u16 BE  ← label
//   bytes  5- 8  JIS Typical Reading (ASCII)       4 bytes
//   bytes  9-512 Image data 64x63 @ 1bpp           504 bytes
//   ETL8B: record ends at byte 512 (total 512 bytes)
//   ETL9B: bytes 513-576 are uncertain            (total 576 bytes)
//
// 0-indexed:
//   [2..4]   -> JIS X 0208 code
//   [8..512] -> image data (504 bytes)
//
// Note: the first record in each ETL8B/9B file is a dummy.
// `read_etl_file` skips it before calling this function.
fn parse_8b(record: &[u8], source_file: Option<String>) -> EtlRecord {
    // Accept both ETL8B (512 bytes) and ETL9B (576 bytes) record sizes.
    assert!(
        record.len() == 512 || record.len() == 576,
        "B-type record must be 512 (ETL8B) or 576 (ETL9B) bytes, got {}",
        record.len()
    );

    // JIS X 0208 code: bytes 3-4 (0-indexed [2..4]), big-endian.
    let jis_code = u16::from_be_bytes([record[2], record[3]]);

    // Image data: bytes 9-512 (0-indexed [8..512]), 504 bytes, 1bpp MSB-first.
    let img_bytes = &record[8..8 + 504];
    let pixels = unpack_1bpp_to_8bit(img_bytes, 64 * 63);

    let character = jis0208_to_char(jis_code);
    let src_stem = source_file
        .as_deref()
        .and_then(|p| {
            std::path::Path::new(p)
                .file_stem()?
                .to_str()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned());
    tracing::debug!(
        source = %src_stem,
        raw_code = format!("0x{jis_code:04X}"),
        char = character.map(|c| c.to_string()).unwrap_or_else(|| "none".to_owned()),
        codepoint = character.map(|c| format!("U+{:04X}", c as u32)).unwrap_or_else(|| "none".to_owned()),
        "loaded class record"
    );

    EtlRecord {
        character,
        raw_code: jis_code,
        pixels,
        width: 64,
        height: 63,
        source_file,
    }
}

// G-type ETL8G / ETL9G - 8199 bytes/record, 128x127 4bpp, JIS X 0208
//
// Official layout (1-indexed, from form_e8g.htm / form_e9g.htm):
//   bytes  1- 2  Serial Sheet Number              u16
//   bytes  3- 4  JIS X 0208 code (2-byte binary)  u16 BE  ← label
//   bytes  5-12  JIS Typical Reading (ASCII)       8 bytes
//   bytes 13-16  Serial Data Number                u32
//   byte  17     Quality of individual image       u8
//   byte  18     Quality of group                  u8
//   byte  19     Male/Female code                  u8
//   byte  20     Age of writer                     u8
//   bytes 21-22  Industry Classification Code      u16
//   bytes 23-24  Occupation Classification Code    u16
//   bytes 25-26  Sheet Gathering Date              u16
//   bytes 27-28  Scanning Date                     u16
//   byte  29     Sample Position X                 u8
//   byte  30     Sample Position Y                 u8
//   ETL8G: bytes 31-60  undefined (30 bytes)  -> image at byte 61 (offset 60)
//   ETL9G: bytes 31-64  undefined (34 bytes)  -> image at byte 65 (offset 64)
//   image: 8128 bytes of 128x127 pixels @ 4bpp
//   ETL8G: bytes 8189-8199 uncertain (11 bytes)
//   ETL9G: bytes 8193-8199 uncertain  (7 bytes)
//
// 0-indexed:
//   [2..4]         -> JIS X 0208 code (direct big-endian u16, same encoding as ETL8B)
//   [img_off..]    -> image (8128 bytes): ETL8G img_off=60, ETL9G img_off=64
//
// The JIS code bytes are raw JIS X 0208 row/col values (range 0x21–0x7e each),
// NOT EUC-JP. EUC-JP would have the high bit set; jis0208_to_char expects the
// plain JIS form already.
fn parse_8g(record: &[u8], img_offset: usize, source_file: Option<String>) -> EtlRecord {
    assert_eq!(record.len(), 8199, "G-type record must be 8199 bytes");

    // JIS X 0208 code: bytes 3-4 (0-indexed [2..4]), big-endian.
    let jis_code = u16::from_be_bytes([record[2], record[3]]);

    // Image data at the format-specific offset: 8128 bytes, 4bpp packed.
    let img_bytes = &record[img_offset..img_offset + 8128];
    let pixels = unpack_4bpp_to_8bit(img_bytes, 128 * 127);

    let character = jis0208_to_char(jis_code);
    let src_stem = source_file
        .as_deref()
        .and_then(|p| {
            std::path::Path::new(p)
                .file_stem()?
                .to_str()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned());
    tracing::debug!(
        source = %src_stem,
        raw_code = format!("0x{jis_code:04X}"),
        char = character.map(|c| c.to_string()).unwrap_or_else(|| "none".to_owned()),
        codepoint = character.map(|c| format!("U+{:04X}", c as u32)).unwrap_or_else(|| "none".to_owned()),
        "loaded class record"
    );

    EtlRecord {
        character,
        raw_code: jis_code,
        pixels,
        width: 128,
        height: 127,
        source_file,
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

/// Convert a JIS X 0201 single byte code to the corresponding Unicode character.
///
/// JIS X 0201 range 0x20–0x7e = ASCII printable with two exceptions:
///   - 0x5c -> U+00A5 YEN SIGN (¥)        not U+005C REVERSE SOLIDUS (\)
///   - 0x7e -> U+203E OVERLINE (‾)         not U+007E TILDE (~)
///
/// Range 0xa1–0xdf = half-width katakana mapped to U+FF61–U+FF9F.
pub fn jis0201_to_char(code: u8) -> Option<char> {
    match code {
        0x5c => Some('¥'), // JIS X 0201: YEN SIGN, not REVERSE SOLIDUS
        0x7e => Some('‾'), // JIS X 0201: OVERLINE, not TILDE
        0x20..=0x7e => char::from_u32(code as u32),
        0xa1..=0xdf => char::from_u32(0xff61 + (code - 0xa1) as u32),
        _ => None,
    }
}

/// Decode a JIS X 0201 byte as hiragana rather than katakana.
///
/// ETL4 stores hiragana images but labels them with JIS X 0201 katakana codes
/// (the encoding has no hiragana range). This function remaps the katakana
/// range (0xa1–0xdf) to the phonetically equivalent hiragana codepoints so
/// that ETL4 records are labelled correctly.
///
/// ASCII printable range (0x20–0x7e) is handled identically to
/// [`jis0201_to_char`], including the 0x5c -> ¥ and 0x7e -> ‾ exceptions.
///
/// Codes with no hiragana equivalent return `None`:
///   - 0xa1–0xa5: punctuation (。「」、・)
///   - 0xb0: katakana-hiragana prolonged sound mark (ー) - katakana-only
///   - 0xde–0xdf: voiced / semi-voiced sound marks (゛゜)
pub fn jis0201_kana_as_hiragana(code: u8) -> Option<char> {
    match code {
        0x5c => Some('¥'),
        0x7e => Some('‾'),
        0x20..=0x7e => char::from_u32(code as u32),
        // 0xa1–0xa5: punctuation - no hiragana equivalent
        0xa6 => Some('を'), // ｦ -> を (U+3092)
        0xa7 => Some('ぁ'), // ｧ -> ぁ (U+3041) small a
        0xa8 => Some('ぃ'), // ｨ -> ぃ (U+3043) small i
        0xa9 => Some('ぅ'), // ｩ -> ぅ (U+3045) small u
        0xaa => Some('ぇ'), // ｪ -> ぇ (U+3047) small e
        0xab => Some('ぉ'), // ｫ -> ぉ (U+3049) small o
        0xac => Some('ゃ'), // ｬ -> ゃ (U+3083) small ya
        0xad => Some('ゅ'), // ｭ -> ゅ (U+3085) small yu
        0xae => Some('ょ'), // ｮ -> ょ (U+3087) small yo
        0xaf => Some('っ'), // ｯ -> っ (U+3063) small tsu
        // 0xb0: prolonged sound mark ｰ - katakana-only, no hiragana equivalent
        0xb1 => Some('あ'), // ｱ -> あ (U+3042)
        0xb2 => Some('い'), // ｲ -> い (U+3044)
        0xb3 => Some('う'), // ｳ -> う (U+3046)
        0xb4 => Some('え'), // ｴ -> え (U+3048)
        0xb5 => Some('お'), // ｵ -> お (U+304A)
        0xb6 => Some('か'), // ｶ -> か (U+304B)
        0xb7 => Some('き'), // ｷ -> き (U+304D)
        0xb8 => Some('く'), // ｸ -> く (U+304F)
        0xb9 => Some('け'), // ｹ -> け (U+3051)
        0xba => Some('こ'), // ｺ -> こ (U+3053)
        0xbb => Some('さ'), // ｻ -> さ (U+3055)
        0xbc => Some('し'), // ｼ -> し (U+3057)
        0xbd => Some('す'), // ｽ -> す (U+3059)
        0xbe => Some('せ'), // ｾ -> せ (U+305B)
        0xbf => Some('そ'), // ｿ -> そ (U+305D)
        0xc0 => Some('た'), // ﾀ -> た (U+305F)
        0xc1 => Some('ち'), // ﾁ -> ち (U+3061)
        0xc2 => Some('つ'), // ﾂ -> つ (U+3064)
        0xc3 => Some('て'), // ﾃ -> て (U+3066)
        0xc4 => Some('と'), // ﾄ -> と (U+3068)
        0xc5 => Some('な'), // ﾅ -> な (U+306A)
        0xc6 => Some('に'), // ﾆ -> に (U+306B)
        0xc7 => Some('ぬ'), // ﾇ -> ぬ (U+306C)
        0xc8 => Some('ね'), // ﾈ -> ね (U+306D)
        0xc9 => Some('の'), // ﾉ -> の (U+306E)
        0xca => Some('は'), // ﾊ -> は (U+306F)
        0xcb => Some('ひ'), // ﾋ -> ひ (U+3072)
        0xcc => Some('ふ'), // ﾌ -> ふ (U+3075)
        0xcd => Some('へ'), // ﾍ -> へ (U+3078)
        0xce => Some('ほ'), // ﾎ -> ほ (U+307B)
        0xcf => Some('ま'), // ﾏ -> ま (U+307E)
        0xd0 => Some('み'), // ﾐ -> み (U+307F)
        0xd1 => Some('む'), // ﾑ -> む (U+3080)
        0xd2 => Some('め'), // ﾒ -> め (U+3081)
        0xd3 => Some('も'), // ﾓ -> も (U+3082)
        0xd4 => Some('や'), // ﾔ -> や (U+3084)
        0xd5 => Some('ゆ'), // ﾕ -> ゆ (U+3086)
        0xd6 => Some('よ'), // ﾖ -> よ (U+3088)
        0xd7 => Some('ら'), // ﾗ -> ら (U+3089)
        0xd8 => Some('り'), // ﾘ -> り (U+308A)
        0xd9 => Some('る'), // ﾙ -> る (U+308B)
        0xda => Some('れ'), // ﾚ -> れ (U+308C)
        0xdb => Some('ろ'), // ﾛ -> ろ (U+308D)
        0xdc => Some('わ'), // ﾜ -> わ (U+308F)
        0xdd => Some('ん'), // ﾝ -> ん (U+3093)
        // 0xde–0xdf: voiced/semi-voiced sound marks - no hiragana equivalent
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

/// Unpack `n` 6-bit values from `src` into 8-bit grayscale, MSB first.
///
/// Six-bit pixels pack into bytes in groups of four: 4 pixels x 6 bits = 24
/// bits = 3 bytes. The pattern within each group of 3 bytes:
///
/// ```text
/// byte 0: [p0 p0 p0 p0 p0 p0 p1 p1]
/// byte 1: [p1 p1 p1 p1 p2 p2 p2 p2]
/// byte 2: [p2 p2 p3 p3 p3 p3 p3 p3]
/// ```
///
/// Each 6-bit value (0–63) is scaled to 0–255 via `v * 255 / 63`.
/// Used exclusively for K-type (ETL2) 60x60 images.
fn unpack_6bpp_to_8bit(src: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    let mut b = 0usize; // byte index into src
    while out.len() < n {
        // Need 3 bytes to decode 4 pixels; stop on short tail.
        if b + 2 >= src.len() {
            break;
        }
        let (b0, b1, b2) = (src[b], src[b + 1], src[b + 2]);
        let p = [
            (b0 >> 2) & 0x3f,
            ((b0 & 0x03) << 4) | (b1 >> 4),
            ((b1 & 0x0f) << 2) | (b2 >> 6),
            b2 & 0x3f,
        ];
        for v in p {
            if out.len() == n {
                break;
            }
            out.push((v as u16 * 255 / 63) as u8);
        }
        b += 3;
    }
    out
}

// C-type ETL3, ETL4, ETL5 - 2952 bytes/record, 72x76 4bpp, JIS X 0201
//
// The record is a stream of 3936 six-bit characters (3936 x 6 = 23616 bits =
// 2952 bytes). Key fields (0-indexed characters -> bit offsets):
//
//   chars 12–17 -> bits  72–107  JIS X 0201 code
//                               top 8 bits (72–79) = byte 9 exactly (72 % 8 = 0)
//   char  288   -> bit  1728     image start (1728 % 8 = 0 -> byte 216)
//                               2736 bytes of 4bpp data -> 72x76 pixels
//
// Because both offsets land on exact byte boundaries, no bit-reader is needed -
// we read byte 9 for the label and call unpack_4bpp_to_8bit on record[216..].
//
// Official layout: <https://etlcdb.db.aist.go.jp/etlcdb/etln/form_c.htm>
// `hiragana` is set for ETL4, which stores hiragana images labelled with JIS
// X 0201 katakana codes. When true, the katakana range is remapped to the
// phonetically equivalent hiragana codepoints via `jis0201_kana_as_hiragana`.
fn parse_ctype(record: &[u8], source_file: Option<String>, hiragana: bool) -> EtlRecord {
    assert_eq!(record.len(), 2952, "C-type record must be 2952 bytes");

    // JIS X 0201 label: top 8 bits of 6-bit chars 12–17 (0-indexed).
    // Bit offset 12*6 = 72; 72/8 = 9 with zero remainder -> plain byte read.
    let jis_byte = record[9];

    // Image: 72x76 pixels at 4bpp starting at bit 1728 (= byte 216).
    // 1728/8 = 216 exactly; 72*76*4/8 = 2736 bytes -> record[216..2952].
    let img_bytes = &record[216..216 + 2736];
    let pixels = unpack_4bpp_to_8bit(img_bytes, 72 * 76);

    let character = if hiragana {
        jis0201_kana_as_hiragana(jis_byte)
    } else {
        jis0201_to_char(jis_byte)
    };
    let src_stem = source_file
        .as_deref()
        .and_then(|p| {
            std::path::Path::new(p)
                .file_stem()?
                .to_str()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned());
    tracing::debug!(
        source = %src_stem,
        raw_code = format!("0x{jis_byte:02X}"),
        hiragana_mode = hiragana,
        char = character.map(|c| c.to_string()).unwrap_or_else(|| "none".to_owned()),
        codepoint = character.map(|c| format!("U+{:04X}", c as u32)).unwrap_or_else(|| "none".to_owned()),
        "loaded class record"
    );

    EtlRecord {
        character,
        raw_code: jis_byte as u16,
        pixels,
        width: 72,
        height: 76,
        source_file,
    }
}

/// Look up a CO-59 grid coordinate `(col, row)` in the AIST mapping table and
/// Look up a CO-59 grid coordinate `(col, row)` in the AIST mapping table and
/// return the corresponding Unicode `char`, or `None` for undefined slots.
fn co59_to_char(col: u8, row: u8) -> Option<char> {
    co59::CO59_TO_UNICODE
        .iter()
        .find(|&&(c, r, _)| c == col && r == row)
        .map(|&(_, _, ch)| ch)
}

// K-type ETL2 - 2745 bytes/record, 60x60 6bpp, CO-59 encoding
//
// The record is a stream of 3660 six-bit characters (3660 x 6 = 21960 bits =
// 2745 bytes). Key fields (0-indexed characters -> bit offsets):
//
//   chars 28–29 -> bits 168–179  CO-59 character code (12 bits, split into two
//                                6-bit halves: col = bits 168–173, row = bits
//                                174–179). 168 % 8 = 0 -> starts at byte 21.
//   char  60    -> bit  360      image start (360 % 8 = 0 -> byte 45)
//                                2700 bytes of 6bpp data -> 60x60 pixels
//
// CO-59 is a proprietary Japanese character encoding used in telegraphy.
// The full 2304-entry CO-59 -> Unicode mapping is implemented in `co59_to_char`
// using the AIST `euc_co59.dat` lookup table.
//
// Official layout: <https://etlcdb.db.aist.go.jp/etlcdb/etln/form_k.htm>
fn parse_ktype(record: &[u8], source_file: Option<String>) -> EtlRecord {
    assert_eq!(record.len(), 2745, "K-type record must be 2745 bytes");

    // CO-59 code: 12-bit value at bits 168–179 (byte 21, exactly aligned).
    // The 12 bits are split into two 6-bit halves:
    //   col = bits 168–173 = record[21] >> 2            (top 6 bits of byte 21)
    //   row = bits 174–179 = (record[21] & 0x03) << 4   (bottom 2 bits of byte 21)
    //                      | record[22] >> 4             (top 4 bits of byte 22)
    let col = record[21] >> 2;
    let row = ((record[21] & 0x03) << 4) | (record[22] >> 4);
    let co59_code = (col as u16) << 6 | row as u16;
    let character = co59_to_char(col, row);

    // Image: 60x60 pixels at 6bpp starting at bit 360 (= byte 45).
    // 360/8 = 45 exactly; 60*60*6/8 = 2700 bytes -> record[45..2745].
    let img_bytes = &record[45..45 + 2700];
    let pixels = unpack_6bpp_to_8bit(img_bytes, 60 * 60);

    let src_stem = source_file
        .as_deref()
        .and_then(|p| {
            std::path::Path::new(p)
                .file_stem()?
                .to_str()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned());
    tracing::debug!(
        source = %src_stem,
        raw_code = format!("0x{co59_code:04X}"),
        col,
        row,
        char = character.map(|c| c.to_string()).unwrap_or_else(|| "none".to_owned()),
        codepoint = character.map(|c| format!("U+{:04X}", c as u32)).unwrap_or_else(|| "none".to_owned()),
        "loaded class record"
    );

    EtlRecord {
        character,
        raw_code: co59_code,
        pixels,
        width: 60,
        height: 60,
        source_file,
    }
}

/// Resize an 8bpp grayscale image from `src_w x src_h` to `dst_w x dst_h`.
///
/// Uses bilinear resampling (Triangle filter) - Lanczos3 produced halos around
/// strokes which was bad for CNN training. Returns a `dst_w * dst_h`-byte
/// row-major `Vec<u8>`.
///
/// This is the general form; [`resize_to_28`] and [`resize_to_64`] are
/// convenience wrappers that return fixed-size arrays.
pub fn resize_image(pixels: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    use image::{GrayImage, imageops};
    let img = GrayImage::from_raw(src_w, src_h, pixels.to_vec())
        .expect("pixel buffer must match src_w x src_h");
    imageops::resize(&img, dst_w, dst_h, imageops::FilterType::Triangle).into_raw()
}

/// Resize a `src_w x src_h` 8bpp grayscale image to 28x28.
#[allow(dead_code)]
///
/// Uses bilinear resampling (Triangle filter). Lanczos3 produced halos around
/// strokes which was bad for CNN training.
///
/// Returns a 784-byte row-major array.
pub fn resize_to_28(pixels: &[u8], src_w: u32, src_h: u32) -> [u8; 784] {
    let v = resize_image(pixels, src_w, src_h, 28, 28);
    let mut out = [0u8; 784];
    out.copy_from_slice(&v);
    out
}

/// Resize a `src_w x src_h` 8bpp grayscale image to 64x64.
#[allow(dead_code)]
///
/// The ETL M-type source images are 64x63 - a near-square at native
/// resolution - so targeting 64x64 preserves far more detail than the
/// old 28x28 MNIST-compatible target. Uses the same Triangle (bilinear)
/// filter as `resize_to_28`.
///
/// Returns a 4096-byte row-major array.
pub fn resize_to_64(pixels: &[u8], src_w: u32, src_h: u32) -> [u8; 4096] {
    let v = resize_image(pixels, src_w, src_h, 64, 64);
    let mut out = [0u8; 4096];
    out.copy_from_slice(&v);
    out
}

/// Write a single NPY v1 array into a deflate-compressed `.npz` file.
///
/// Thin wrapper around [`lib::npz::write_npz`] - the canonical implementation
/// lives in the shared `lib` crate so `mitchty` can use it too without
/// duplicating the format logic.
pub fn write_npz(path: &str, data: &[u8], shape: &[usize], dtype: &str) -> io::Result<()> {
    lib::npz::write_npz(path, data, shape, dtype)
}

/// Convert all ETL batches from [`read_etlcdb_dir`] into npz files.
///
/// Each batch (M-type, B-type, G-type) is written to its own set of npz files
/// because the native image dimensions differ between format families and a
/// single npz array must have a uniform shape.
///
/// ## Output layout
///
/// Without `--train-split` (`train_fraction == 0.0`):
/// ```text
/// {out_dir}/m-imgs.npz   {out_dir}/m-labels.npz
/// {out_dir}/b-imgs.npz   {out_dir}/b-labels.npz
/// {out_dir}/g-imgs.npz   {out_dir}/g-labels.npz
/// {out_dir}/classmap.json
/// {out_dir}/stats.json
/// ```
///
/// With `--train-split` (e.g. 0.8):
/// ```text
/// {out_dir}/m-train-imgs.npz   {out_dir}/m-train-labels.npz
/// {out_dir}/m-test-imgs.npz    {out_dir}/m-test-labels.npz
/// … (same for b- and g- prefixes)
/// {out_dir}/classmap.json
/// {out_dir}/stats.json
/// ```
///
/// The `classmap.json` and `stats.json` are shared across all batches - class
/// indices are globally consistent so you can train on M+B+G together by
/// passing all the npz files to `ma train`.
pub fn convert_etlcdb(
    batches: &[EtlBatch],
    out_dir: &str,
    train_fraction: f64,
    filter_chars: &[char],
    filter_names: &[String],
    equiv: &HashMap<char, char>,
) -> io::Result<()> {
    if batches.is_empty() {
        tracing::warn!("no ETL batches found, nothing to write");
        return Ok(());
    }

    // Build a single shared label map from ALL records across all batches so
    // that class indices are globally consistent between the per-family npz files.
    // Apply character filters first so excluded chars never appear in the label map.
    // equiv is passed through so halfwidth chars normalise to the same canonical
    // label as their fullwidth counterparts without mutating any records.
    let all_records: Vec<&EtlRecord> = batches
        .iter()
        .flat_map(|b| b.records.iter())
        .filter(|r| {
            r.character
                .map(|ch| filter_reason(ch, filter_chars, filter_names).is_none())
                .unwrap_or(false)
        })
        .collect();

    if !filter_chars.is_empty() || !filter_names.is_empty() {
        let total: usize = batches.iter().map(|b| b.records.len()).sum();
        let kept = all_records.len();
        tracing::info!(
            total,
            kept,
            removed = total - kept,
            "character filters applied"
        );
    }

    // Build the label map directly from the filtered record references -
    // no clone of the entire record corpus is needed.
    let label_map = label_map_from_record_refs(&all_records, equiv);
    tracing::info!(
        classes = label_map.len(),
        "global label map built from all ETL batches"
    );

    // Write shared classmap.json.
    write_classmap(out_dir, &label_map)?;

    // Compute stats via a streaming Welford pass - no all_pixels Vec needed.
    let mut stats_acc = WelfordStats::default();
    let mut stats_n_images = 0usize;
    for r in &all_records {
        if let Some(ch) = r.character {
            let canonical = crate::kana_merging::equiv_char(ch, equiv);
            if label_map.contains_key(&canonical) {
                stats_acc.update(&r.pixels);
                stats_n_images += 1;
            }
        }
    }
    // Drop all_records now - we only need label_map going forward.
    drop(all_records);
    let (stats_mean, stats_std) = stats_acc.finish();
    write_stats_computed(out_dir, stats_mean, stats_std, stats_n_images)?;

    // Write per-batch npz files.
    let num_classes = label_map.len();
    let use_u16 = num_classes > 255;
    let label_stride = if use_u16 { 2 } else { 1 };
    let label_dtype = if use_u16 { "<u2" } else { "|u1" };

    for batch in batches {
        // Collect all records for this batch with their canonical char, sorted
        // for deterministic output order. All records are kept (multiple writers
        // per character all get their own label+pixel entry).
        // Filter is re-applied here on the original char so that records whose
        // original char was excluded cannot leak images into a class that shares
        // the same equiv-canonical as an included char.
        let mut batch_entries: Vec<(char, &EtlRecord)> = batch
            .records
            .iter()
            .filter_map(|r| {
                let ch = r.character?;
                if filter_reason(ch, filter_chars, filter_names).is_some() {
                    return None;
                }
                Some((crate::kana_merging::equiv_char(ch, equiv), r))
            })
            .collect();
        batch_entries.sort_by_key(|(ch, _)| *ch);

        // Pre-count accepted records so images Vec can be pre-allocated exactly.
        let accepted: Vec<(char, &EtlRecord)> = batch_entries
            .into_iter()
            .filter(|(ch, _)| label_map.contains_key(ch))
            .collect();

        let n = accepted.len();
        if n == 0 {
            tracing::warn!(tag = batch.tag, "no matching records in batch, skipping");
            continue;
        }

        let ppi_hint = accepted
            .first()
            .map(|(_, r)| r.width as usize * r.height as usize)
            .unwrap_or(0);
        let mut images: Vec<u8> = Vec::with_capacity(n * ppi_hint);
        let mut labels_u32: Vec<u32> = Vec::with_capacity(n);
        let mut img_width: u32 = 0;
        let mut img_height: u32 = 0;

        for (ch, rec) in &accepted {
            if let Some(&lbl) = label_map.get(ch) {
                if img_width == 0 {
                    img_width = rec.width;
                    img_height = rec.height;
                }
                images.extend_from_slice(&rec.pixels);
                labels_u32.push(lbl);
            }
        }

        let n = labels_u32.len();
        if n == 0 {
            tracing::warn!(tag = batch.tag, "no matching records in batch, skipping");
            continue;
        }

        let label_bytes: Vec<u8> = if use_u16 {
            labels_u32
                .iter()
                .flat_map(|&l| (l as u16).to_le_bytes())
                .collect()
        } else {
            labels_u32.iter().map(|&l| l as u8).collect()
        };

        let h = img_height as usize;
        let w = img_width as usize;
        let ppi = h * w;

        // Derive output paths with the per-batch tag prefix.
        let p = |name: &str| format!("{out_dir}/{}-{name}", batch.tag);
        let paths = if train_fraction > 0.0 {
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
        };

        tracing::info!(
            tag = batch.tag,
            n,
            classes = num_classes,
            label_dtype,
            img_height = h,
            img_width = w,
            "writing batch npz files"
        );

        match (&paths.test_imgs, &paths.test_labels) {
            (Some(test_imgs), Some(test_labels)) => {
                let train_n = ((n as f64 * train_fraction).round() as usize).clamp(1, n - 1);
                write_npz(
                    &paths.imgs,
                    &images[..train_n * ppi],
                    &[train_n, h, w],
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
                    &images[train_n * ppi..],
                    &[n - train_n, h, w],
                    "|u1",
                )?;
                write_npz(
                    test_labels,
                    &label_bytes[train_n * label_stride..],
                    &[n - train_n],
                    label_dtype,
                )?;
                tracing::info!(
                    tag = batch.tag,
                    train = train_n,
                    test = n - train_n,
                    "split written"
                );
            }
            _ => {
                write_npz(&paths.imgs, &images, &[n, h, w], "|u1")?;
                write_npz(&paths.labels, &label_bytes, &[n], label_dtype)?;
                tracing::info!(tag = batch.tag, n, "written");
            }
        }
    }

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

/// Convert ETL records to npz files using `OutputPaths`.
///
/// Images are stored at their **native resolution** - no resizing is applied.
/// All records within a single ETL file share the same dimensions (e.g. 64x63
/// for M/B-type, 128x127 for G-type), so the npz shape is derived from the
/// first accepted record. Resize to whatever target size you need at training
/// time in the dataloader - baking a resize into the stored data would throw
/// away information you can never recover.
///
/// Image array shape: `[N, height, width]` - row-major, C-order, same as
/// numpy default.
///
/// `label_map` maps Unicode char -> u32 class index. Records whose character
/// is absent from the map, or could not be decoded, are silently dropped.
///
/// Labels are written as `|u1` (1 byte) when num_classes ≤ 255, and as `<u2`
/// (2-byte little-endian u16) when num_classes > 255.
///
/// When `paths.test_imgs` / `paths.test_labels` are `Some`, the data is split
/// at `train_n = round(total * train_fraction)` items.
pub fn convert_to_npz(
    records: &[EtlRecord],
    paths: &OutputPaths,
    label_map: &HashMap<char, u32>,
    train_fraction: f64,
    equiv: &HashMap<char, char>,
) -> io::Result<()> {
    let mut images: Vec<u8> = Vec::new();
    let mut labels_u32: Vec<u32> = Vec::new();
    // Dimensions come from the first accepted record; all records in an ETL
    // file share the same native size so this is always consistent.
    let mut img_width: u32 = 0;
    let mut img_height: u32 = 0;

    for rec in records {
        let Some(ch) = rec.character else { continue };
        let canonical = crate::kana_merging::equiv_char(ch, equiv);
        let Some(&label) = label_map.get(&canonical) else {
            continue;
        };
        if img_width == 0 {
            img_width = rec.width;
            img_height = rec.height;
        }
        images.extend_from_slice(&rec.pixels);
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
    let h = img_height as usize;
    let w = img_width as usize;
    let ppi = h * w; // pixels per image at native resolution

    write_classmap(out_dir, label_map)?;
    write_stats(out_dir, &images, ppi)?;

    tracing::info!(
        n,
        classes = num_classes,
        label_dtype,
        img_height = h,
        img_width = w,
        "writing npz files"
    );

    // Bytes per label in its serialised form
    let label_stride = if use_u16 { 2 } else { 1 };

    match (&paths.test_imgs, &paths.test_labels) {
        (Some(test_imgs), Some(test_labels)) => {
            let train_n = ((n as f64 * train_fraction).round() as usize).clamp(1, n - 1);

            write_npz(
                &paths.imgs,
                &images[..train_n * ppi],
                &[train_n, h, w],
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
                &images[train_n * ppi..],
                &[n - train_n, h, w],
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
            write_npz(&paths.imgs, &images, &[n, h, w], "|u1")?;
            write_npz(&paths.labels, &label_bytes, &[n], label_dtype)?;
            tracing::info!(n, imgs = %paths.imgs, labels = %paths.labels, "written");
        }
    }

    Ok(())
}

/// Merge all ETL batches into a **single** set of npz files at a common square target size.
///
/// Because the native image dimensions differ between format families (64x63
/// for M/B-type, 128x127 for G-type), merging into one uniform `[N, H, W]`
/// array requires resizing every image to `dst_w x dst_h`. Images that are
/// already at the target size are copied as-is; others are upscaled or
/// downscaled using bilinear (Triangle) resampling.
///
/// Pass the values returned by [`max_batch_dims`] so that `dst_w == dst_h` and
/// the output tensors are always **square** (e.g. 128x128 for a full ETLcdb run).
/// Square images are required by most CNN training pipelines and allow the merged
/// dataset to be trivially concatenated with other square datasets (KanjiVG 28x28, etc.).
///
/// ## Output layout
///
/// Without `--train-split` (`train_fraction == 0.0`):
/// ```text
/// {out_dir}/imgs.npz     {out_dir}/labels.npz
/// {out_dir}/classmap.json
/// {out_dir}/stats.json   (computed on the *target-resolution* pixels)
/// ```
///
/// With `--train-split` (e.g. 0.8):
/// ```text
/// {out_dir}/train-imgs.npz   {out_dir}/train-labels.npz
/// {out_dir}/test-imgs.npz    {out_dir}/test-labels.npz
/// {out_dir}/classmap.json
/// {out_dir}/stats.json
/// ```
///
/// Identical output layout to a `kanjivg` conversion.
#[allow(clippy::too_many_arguments)]
pub fn write_merged_etlcdb(
    batches: &[EtlBatch],
    out_dir: &str,
    train_fraction: f64,
    dst_w: u32,
    dst_h: u32,
    filter_chars: &[char],
    filter_names: &[String],
    equiv: &HashMap<char, char>,
) -> io::Result<()> {
    if batches.is_empty() {
        tracing::warn!("no ETL batches found, nothing to write");
        return Ok(());
    }

    let all_records: Vec<&EtlRecord> = batches
        .iter()
        .flat_map(|b| b.records.iter())
        .filter(|r| {
            r.character
                .map(|ch| filter_reason(ch, filter_chars, filter_names).is_none())
                .unwrap_or(false)
        })
        .collect();

    if !filter_chars.is_empty() || !filter_names.is_empty() {
        let total: usize = batches.iter().map(|b| b.records.len()).sum();
        let kept = all_records.len();
        tracing::info!(
            total,
            kept,
            removed = total - kept,
            "character filters applied"
        );
    }

    // Build label map from references - no clone of the record corpus needed.
    let label_map = label_map_from_record_refs(&all_records, equiv);
    tracing::info!(
        classes = label_map.len(),
        dst_w,
        dst_h,
        "merging all ETL batches into single npz set"
    );
    // Drop the reference vec; label_map is all we need going forward.
    drop(all_records);

    let num_classes = label_map.len();
    let use_u16 = num_classes > 255;
    let label_dtype = if use_u16 { "<u2" } else { "|u1" };
    let label_stride = if use_u16 { 2 } else { 1 };
    let out_w = dst_w as usize;
    let out_h = dst_h as usize;
    let ppi = out_w * out_h;

    // Pre-count accepted records across all batches so images Vec can be
    // pre-allocated exactly - avoids all reallocation copies.
    let total_accepted: usize = batches
        .iter()
        .flat_map(|b| b.records.iter())
        .filter(|r| {
            r.character
                .map(|ch| {
                    filter_reason(ch, filter_chars, filter_names).is_none()
                        && label_map.contains_key(&crate::kana_merging::equiv_char(ch, equiv))
                })
                .unwrap_or(false)
        })
        .count();

    let total_records: u64 = batches.iter().map(|b| b.records.len() as u64).sum();
    let pb = record_progress_bar(total_records, "resizing");

    let mut images: Vec<u8> = Vec::with_capacity(total_accepted * ppi);
    let mut labels_u32: Vec<u32> = Vec::with_capacity(total_accepted);
    // Welford stats accumulator - no separate all_pixels Vec needed.
    let mut stats_acc = WelfordStats::default();

    for batch in batches {
        let src_w = batch.records.first().map(|r| r.width).unwrap_or(dst_w);
        let src_h = batch.records.first().map(|r| r.height).unwrap_or(dst_h);
        let needs_resize = src_w != dst_w || src_h != dst_h;
        let tag = batch.tag;

        pb.set_message(format!("{tag}: {}x{} -> {}x{}", src_w, src_h, dst_w, dst_h));

        // Per-batch class contribution counter - only populated at debug log
        // level so it is free at info level. Reveals which batch injects
        // unexpected character classes (e.g. katakana appearing in a batch
        // documented as hiragana-only).
        let mut batch_class_counts: std::collections::HashMap<char, u32> =
            std::collections::HashMap::new();
        // Per-source-file breakdown: file stem -> (canonical_char -> count).
        // Only built at debug level. Lets us pinpoint which specific ETL file
        // (e.g. ETL5_1) is contributing unexpected character images.
        let mut file_class_counts: std::collections::HashMap<
            String,
            std::collections::HashMap<char, u32>,
        > = std::collections::HashMap::new();

        for rec in &batch.records {
            let Some(ch) = rec.character else {
                pb.inc(1);
                continue;
            };
            // Re-apply the filter on the original char here, not just during
            // label_map construction. Without this, records whose original char
            // was filtered out can still leak through if their equiv-canonical
            // is already in the label_map from another non-filtered record.
            if filter_reason(ch, filter_chars, filter_names).is_some() {
                pb.inc(1);
                continue;
            }
            let canonical = crate::kana_merging::equiv_char(ch, equiv);
            let Some(&lbl) = label_map.get(&canonical) else {
                pb.inc(1);
                continue;
            };

            *batch_class_counts.entry(canonical).or_insert(0) += 1;
            if tracing::enabled!(tracing::Level::DEBUG) {
                let file_key = rec
                    .source_file
                    .as_deref()
                    .and_then(|p| {
                        std::path::Path::new(p)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_owned())
                    })
                    .unwrap_or_else(|| "unknown".to_owned());
                *file_class_counts
                    .entry(file_key)
                    .or_default()
                    .entry(canonical)
                    .or_insert(0) += 1;
            }

            if needs_resize {
                let resized = resize_image(&rec.pixels, src_w, src_h, dst_w, dst_h);
                stats_acc.update(&resized);
                images.extend_from_slice(&resized);
            } else {
                stats_acc.update(&rec.pixels);
                images.extend_from_slice(&rec.pixels);
            }
            labels_u32.push(lbl);
            pb.inc(1);
        }

        // Log per-batch class breakdown at debug level. Run with
        // RUST_LOG=ma=debug to see which batches contribute which characters.
        // At info level just log the unique class count so there's no noise.
        let batch_classes = batch_class_counts.len();
        let batch_records = batch_class_counts.values().copied().sum::<u32>();
        tracing::info!(
            tag,
            unique_classes = batch_classes,
            records_written = batch_records,
            src_w,
            src_h,
            "batch contribution"
        );

        // Warn when a B/G-type batch (documented hiragana+kanji, JIS X 0208)
        // unexpectedly contains katakana records - that would indicate mislabeled
        // source data since those encodings don't include a katakana range.
        // M-type (ETL7) and C-type (ETL4) hiragana is intentional: those datasets
        // store hiragana images under JIS X 0201 katakana codes and are remapped
        // at parse time via jis0201_kana_as_hiragana - no warning needed there.
        let katakana_count: u32 = batch_class_counts
            .iter()
            .filter(|(ch, _)| ('\u{30A0}'..='\u{30FF}').contains(*ch))
            .map(|(_, n)| n)
            .sum();
        if (tag == "b" || tag == "g") && katakana_count > 0 {
            tracing::warn!(
                tag,
                katakana_records = katakana_count,
                "B/G-type batch (documented hiragana+kanji) contains katakana records - \
                 check source ETL files for mislabeled data"
            );
        }
        // Both C-type (ETL4) and M-type (ETL7) hiragana are expected: those
        // datasets store hiragana images under JIS X 0201 katakana codes and
        // are intentionally remapped via jis0201_kana_as_hiragana at parse time.

        if tracing::enabled!(tracing::Level::DEBUG) {
            // Per-batch char breakdown sorted by codepoint.
            let mut pairs: Vec<(char, u32)> = batch_class_counts.into_iter().collect();
            pairs.sort_by_key(|(ch, _)| *ch as u32);
            for (ch, count) in &pairs {
                let cp = *ch as u32;
                let name = unicode_names2::name(*ch)
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                tracing::debug!(
                    tag,
                    char = %ch,
                    codepoint = format!("U+{cp:04X}"),
                    unicode_name = %name,
                    count,
                    "batch class record"
                );
            }

            // Per-source-file breakdown: file -> char -> count, sorted by file
            // name then codepoint. Pinpoints exactly which ETL file contributes
            // unexpected images (e.g. ETL5_1 writing hiragana ん under ﾝ code).
            let mut file_pairs: Vec<(String, Vec<(char, u32)>)> = file_class_counts
                .into_iter()
                .map(|(f, m)| {
                    let mut cv: Vec<(char, u32)> = m.into_iter().collect();
                    cv.sort_by_key(|(ch, _)| *ch as u32);
                    (f, cv)
                })
                .collect();
            file_pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
            for (file, char_counts) in &file_pairs {
                let total: u32 = char_counts.iter().map(|(_, n)| n).sum();
                tracing::debug!(tag, file, total_records = total, "source file contribution");
                for (ch, count) in char_counts {
                    let cp = *ch as u32;
                    tracing::debug!(
                        tag,
                        file,
                        char = %ch,
                        codepoint = format!("U+{cp:04X}"),
                        count,
                        "source file class record"
                    );
                }
            }
        }
    }
    pb.finish_with_message(format!(
        "resized {} records -> {} images",
        HumanCount(total_records),
        HumanCount(labels_u32.len() as u64),
    ));

    let n = labels_u32.len();
    if n == 0 {
        tracing::warn!("no matching records found, nothing written");
        return Ok(());
    }

    let label_bytes: Vec<u8> = if use_u16 {
        labels_u32
            .iter()
            .flat_map(|&l| (l as u16).to_le_bytes())
            .collect()
    } else {
        labels_u32.iter().map(|&l| l as u8).collect()
    };

    write_classmap(out_dir, &label_map)?;
    // Stats computed via streaming Welford accumulator above - no separate
    // pixel buffer needed.
    let (stats_mean, stats_std) = stats_acc.finish();
    let stats_n = images.len().checked_div(ppi).unwrap_or(0);
    write_stats_computed(out_dir, stats_mean, stats_std, stats_n)?;

    let write_pb = write_progress_bar(images.len() as u64, "writing imgs.npz");
    tracing::info!(
        n,
        classes = num_classes,
        label_dtype,
        dst_w,
        dst_h,
        "writing merged npz files"
    );

    let paths = output_paths(out_dir, train_fraction);

    match (&paths.test_imgs, &paths.test_labels) {
        (Some(test_imgs), Some(test_labels)) => {
            let train_n = ((n as f64 * train_fraction).round() as usize).clamp(1, n - 1);

            write_pb.set_message(format!("writing {}", &paths.imgs));
            write_pb.set_length((images.len() + label_bytes.len()) as u64);
            write_npz(
                &paths.imgs,
                &images[..train_n * ppi],
                &[train_n, out_h, out_w],
                "|u1",
            )?;
            write_pb.set_position((train_n * ppi) as u64);
            write_pb.set_message(format!("writing {}", &paths.labels));
            write_npz(
                &paths.labels,
                &label_bytes[..train_n * label_stride],
                &[train_n],
                label_dtype,
            )?;
            write_pb.set_message(format!("writing {test_imgs}"));
            write_npz(
                test_imgs,
                &images[train_n * ppi..],
                &[n - train_n, out_h, out_w],
                "|u1",
            )?;
            write_pb.set_position(images.len() as u64);
            write_pb.set_message(format!("writing {test_labels}"));
            write_npz(
                test_labels,
                &label_bytes[train_n * label_stride..],
                &[n - train_n],
                label_dtype,
            )?;
            write_pb.finish_with_message(format!(
                "wrote train={} test={} images ({})",
                HumanCount(train_n as u64),
                HumanCount((n - train_n) as u64),
                HumanBytes(images.len() as u64),
            ));
            tracing::info!(train = train_n, test = n - train_n, "merged split written");
        }
        _ => {
            write_pb.set_message(format!("writing {}", &paths.imgs));
            write_pb.set_length(images.len() as u64);
            write_npz(&paths.imgs, &images, &[n, out_h, out_w], "|u1")?;
            write_pb.set_position(images.len() as u64);
            write_pb.set_message(format!("writing {}", &paths.labels));
            write_npz(&paths.labels, &label_bytes, &[n], label_dtype)?;
            write_pb.finish_with_message(format!(
                "wrote {} images ({})",
                HumanCount(n as u64),
                HumanBytes(images.len() as u64),
            ));
            tracing::info!(n, imgs = %paths.imgs, labels = %paths.labels, "merged written");
        }
    }

    Ok(())
}

/// Return the square power-of-two target dimension for merging all batches.
///
/// Takes the maximum width and height across every record, picks the larger
/// of the two, then rounds up to the next power of two. The same value is
/// used for both width and height so the merged npz always contains **square**
/// images - a requirement for most CNN training pipelines.
///
/// ## Standard ETL dataset results
///
/// | Largest native size | max(w,h) | rounded up -> |
/// |---------------------|----------|--------------|
/// | G-type 128 x 127    | 128      | **128 x 128** |
/// | M/B-type 64 x 63    | 64       | **64 x 64**   |
///
/// Note: 128 x 127 is the *real* hardware sensor resolution of ETL8G/ETL9G -
/// not an off-by-one bug. We deliberately round up to 128 x 128 so that the
/// merged output tensor is square.
pub fn max_batch_dims(batches: &[EtlBatch]) -> (u32, u32) {
    let (mw, mh) = batches
        .iter()
        .flat_map(|b| b.records.iter())
        .fold((1u32, 1u32), |(mw, mh), r| {
            (mw.max(r.width), mh.max(r.height))
        });
    let side = mw.max(mh).next_power_of_two();
    (side, side)
}

// ---------------------------------------------------------------------------
// Progress-bar helpers
// ---------------------------------------------------------------------------

/// A spinner-style bar for counting records processed (unknown compressed size).
fn record_progress_bar(total: u64, verb: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.cyan} [{elapsed_precise}] {bar:45.cyan/blue} {human_pos}/{human_len} \
             records {per_sec:.dim} eta {eta} {msg}",
        )
        .unwrap()
        .progress_chars("█▉▊▋▌▍▎▏  "),
    );
    pb.set_message(verb.to_owned());
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}

/// A byte-based bar for tracking npz write progress.
fn write_progress_bar(total_bytes: u64, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(total_bytes);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] {bar:45.green/dim} {bytes}/{total_bytes} \
             {bytes_per_sec:.dim} eta {eta} {msg}",
        )
        .unwrap()
        .progress_chars("█▉▊▋▌▍▎▏  "),
    );
    pb.set_message(msg.to_owned());
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}

/// Write `{out_dir}/classmap.json` a glorified JSON array of single-char strings in
/// label-index order, e.g. `["あ","い","う",…,"一","二",…]`.
///
/// The array index equals the class label used in the npz files, so
/// `classmap[i]` is the Unicode character for class `i`. This file is
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

    // Serialize via serde_json so special characters (", \, control chars)
    // are properly escaped - hand-rolling format!("\"{}\"", c) does not escape.
    let char_strings: Vec<String> = chars.iter().map(|c| c.to_string()).collect();
    let json = serde_json::to_string(&char_strings)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
        + "\n";

    let path = format!("{out_dir}/classmap.json");
    fs::write(&path, json.as_bytes())?;
    tracing::info!(path, classes = n, "classmap written");
    Ok(())
}

/// Online Welford accumulator for per-pixel mean and variance.
///
/// Accumulates statistics one pixel slice at a time so callers never need to
/// materialise a flat all-pixels buffer just to compute `stats.json`. All
/// arithmetic is in `f64`; pixel values are normalized to `[0, 1]` on the fly.
///
/// # Example
///
/// ```rust
/// let mut acc = WelfordStats::default();
/// for rec in &records {
///     acc.update(&rec.pixels);
/// }
/// let (mean, std) = acc.finish();
/// ```
#[derive(Debug, Default, Clone)]
pub struct WelfordStats {
    count: f64,
    mean: f64,
    m2: f64,
}

impl WelfordStats {
    /// Feed one pixel slice (any length) into the accumulator.
    ///
    /// Pixel values are scaled to `[0, 1]` before accumulation.
    pub fn update(&mut self, pixels: &[u8]) {
        for &v in pixels {
            self.count += 1.0;
            let x = v as f64 / 255.0;
            let delta = x - self.mean;
            self.mean += delta / self.count;
            let delta2 = x - self.mean;
            self.m2 += delta * delta2;
        }
    }

    /// Return `(mean, std)` both in `[0, 1]`.
    ///
    /// Returns `(0.0, 0.0)` if no pixels were fed.
    pub fn finish(&self) -> (f64, f64) {
        if self.count < 2.0 {
            return (self.mean, 0.0);
        }
        let variance = self.m2 / self.count; // population variance
        (self.mean, variance.sqrt())
    }

    /// Number of pixels accumulated so far.
    #[allow(dead_code)]
    pub fn pixel_count(&self) -> u64 {
        self.count as u64
    }
}

/// Write `{out_dir}/stats.json` with per-pixel mean and std over the whole corpus.
///
/// `images` is the flat `u8` pixel buffer, `pixels_per_image` is `height * width`
/// for whatever native resolution the dataset uses. Stats are computed over all
/// pixels at once, values scaled to `[0, 1]`, for use as normalisation constants
/// at training/inference time.
///
/// For streaming use (no large pixel buffer), prefer [`write_stats_computed`]
/// together with [`WelfordStats`].
pub fn write_stats(out_dir: &str, images: &[u8], pixels_per_image: usize) -> io::Result<()> {
    let mut acc = WelfordStats::default();
    acc.update(images);
    let (mean, std) = acc.finish();
    let n_images = images.len().checked_div(pixels_per_image).unwrap_or(0);
    write_stats_computed(out_dir, mean, std, n_images)
}

/// Write `{out_dir}/stats.json` from already-computed mean, std, and image count.
///
/// Use this together with [`WelfordStats`] when you want to compute statistics
/// in a streaming fashion without materialising a flat pixel buffer.
pub fn write_stats_computed(out_dir: &str, mean: f64, std: f64, n_images: usize) -> io::Result<()> {
    if n_images == 0 {
        return Ok(());
    }
    let json = format!(
        "{{\n  \"norm_mean\": {:.6},\n  \"norm_std\": {:.6},\n  \"n_images\": {}\n}}\n",
        mean, std, n_images,
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
///
/// `equiv` is applied to each character before deduplication and index
/// assignment, so halfwidth and fullwidth variants of the same character
/// collapse to a single canonical label without mutating any records.
/// Pass an empty map when no equivalence merging is needed.
pub fn label_map_from_records(
    records: &[EtlRecord],
    equiv: &HashMap<char, char>,
) -> HashMap<char, u32> {
    let mut seen: Vec<char> = Vec::new();
    for rec in records {
        if let Some(ch) = rec.character {
            let canonical = crate::kana_merging::equiv_char(ch, equiv);
            if !seen.contains(&canonical) {
                seen.push(canonical);
            }
        }
    }
    seen.sort_unstable();
    build_label_map(&seen)
}

/// Like [`label_map_from_records`] but accepts a slice of references, avoiding
/// the need to clone the record corpus just to build the label map.
pub(crate) fn label_map_from_record_refs(
    records: &[&EtlRecord],
    equiv: &HashMap<char, char>,
) -> HashMap<char, u32> {
    let mut seen: Vec<char> = Vec::new();
    for rec in records {
        if let Some(ch) = rec.character {
            let canonical = crate::kana_merging::equiv_char(ch, equiv);
            if !seen.contains(&canonical) {
                seen.push(canonical);
            }
        }
    }
    seen.sort_unstable();
    build_label_map(&seen)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Bit/nibble unpacking primitives
    // -----------------------------------------------------------------------

    #[test]
    fn unpack_4bpp_nibble_scaling() {
        // 0x0f -> hi=0, lo=15 -> 0*17=0, 15*17=255
        let src = [0x0f_u8];
        let out = unpack_4bpp_to_8bit(&src, 2);
        assert_eq!(out, vec![0, 255]);
    }

    #[test]
    fn unpack_4bpp_mid_value() {
        // 0x80 -> hi=8, lo=0 -> 8*17=136, 0*17=0
        let src = [0x80_u8];
        let out = unpack_4bpp_to_8bit(&src, 2);
        assert_eq!(out, vec![136, 0]);
    }

    #[test]
    fn unpack_4bpp_truncates_to_n() {
        // Two bytes -> 4 nibbles but we only want 3
        let src = [0x12_u8, 0x34];
        let out = unpack_4bpp_to_8bit(&src, 3);
        assert_eq!(out.len(), 3);
        assert_eq!(out, vec![17, 2 * 17, 3 * 17]);
    }

    #[test]
    fn unpack_1bpp_msb_first() {
        // 0b10110000 = 0xb0 -> 1,0,1,1,0,0,0,0 -> 255,0,255,255,0,0,0,0
        let src = [0xb0_u8];
        let out = unpack_1bpp_to_8bit(&src, 8);
        assert_eq!(out, vec![255, 0, 255, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn unpack_1bpp_truncates_to_n() {
        let src = [0xff_u8, 0x00];
        let out = unpack_1bpp_to_8bit(&src, 10);
        assert_eq!(out.len(), 10);
        // First 8 bits all 1 -> 255; next 2 bits all 0 -> 0
        assert_eq!(&out[..8], &[255u8; 8]);
        assert_eq!(&out[8..], &[0u8; 2]);
    }

    // -----------------------------------------------------------------------
    // JIS X 0201 decoder
    // -----------------------------------------------------------------------

    #[test]
    fn jis0201_ascii_range() {
        // 0x41 = 'A' in ASCII / JIS X 0201
        assert_eq!(jis0201_to_char(0x41), Some('A'));
        // 0x30 = '0'
        assert_eq!(jis0201_to_char(0x30), Some('0'));
        // 0x20 = space
        assert_eq!(jis0201_to_char(0x20), Some(' '));
    }

    #[test]
    fn jis0201_yen_sign_not_backslash() {
        // JIS X 0201 maps 0x5c to YEN SIGN (U+00A5), not REVERSE SOLIDUS (U+005C).
        // ETL1/3/4/5/6/7 records with a yen image will have JIS code 0x5c; they
        // must decode to '¥' so the label map and output character are correct.
        assert_eq!(jis0201_to_char(0x5c), Some('¥'));
        assert_ne!(jis0201_to_char(0x5c), Some('\\'));
    }

    #[test]
    fn jis0201_overline_not_tilde() {
        // JIS X 0201 maps 0x7e to OVERLINE (U+203E), not TILDE (U+007E).
        assert_eq!(jis0201_to_char(0x7e), Some('‾'));
        assert_ne!(jis0201_to_char(0x7e), Some('~'));
    }

    #[test]
    fn jis0201_katakana_range() {
        // 0xa1 -> U+FF61 HALFWIDTH IDEOGRAPHIC FULL STOP (｡)
        assert_eq!(jis0201_to_char(0xa1), char::from_u32(0xFF61));
        // 0xb1 -> U+FF71 HALFWIDTH KATAKANA LETTER A (ｱ)
        assert_eq!(jis0201_to_char(0xb1), char::from_u32(0xFF71));
        // 0xdf -> U+FF9F HALFWIDTH KATAKANA VOICED ITERATION MARK (ﾟ)
        assert_eq!(jis0201_to_char(0xdf), char::from_u32(0xFF9F));
    }

    #[test]
    fn jis0201_out_of_range_is_none() {
        assert_eq!(jis0201_to_char(0x00), None);
        assert_eq!(jis0201_to_char(0x1f), None); // below 0x20
        assert_eq!(jis0201_to_char(0x7f), None); // DEL, not in 0x20–0x7e
        assert_eq!(jis0201_to_char(0x80), None); // gap between ASCII and katakana
        assert_eq!(jis0201_to_char(0xe0), None); // above katakana range
    }

    // -----------------------------------------------------------------------
    // JIS X 0208 decoder
    // -----------------------------------------------------------------------

    #[test]
    fn jis0208_katakana_small_ya_yo_yu_tu() {
        // JIS X 0208 katakana layout (EUC-JP verified):
        // Row 0x25 encodes the katakana block in sequential Unicode order.
        // Small forms come before their large counterparts at odd offsets.
        //
        // 0x2563 -> ャ (KATAKANA LETTER SMALL YA)
        // 0x2564 -> ヤ (KATAKANA LETTER YA)
        // 0x2565 -> ュ (KATAKANA LETTER SMALL YU)
        // 0x2566 -> ユ (KATAKANA LETTER YU)
        // 0x2567 -> ョ (KATAKANA LETTER SMALL YO)
        // 0x2568 -> ヨ (KATAKANA LETTER YO)
        // 0x2543 -> ッ (KATAKANA LETTER SMALL TU)
        assert_eq!(jis0208_to_char(0x2563), Some('ャ'), "ャ small ya");
        assert_eq!(jis0208_to_char(0x2564), Some('ヤ'), "ヤ ya");
        assert_eq!(jis0208_to_char(0x2565), Some('ュ'), "ュ small yu");
        assert_eq!(jis0208_to_char(0x2566), Some('ユ'), "ユ yu");
        assert_eq!(jis0208_to_char(0x2567), Some('ョ'), "ョ small yo");
        assert_eq!(jis0208_to_char(0x2568), Some('ヨ'), "ヨ yo");
        assert_eq!(jis0208_to_char(0x2543), Some('ッ'), "ッ small tu");
    }

    #[test]
    fn jis0208_hiragana_a() {
        // JIS X 0208 row 0x24, col 0x22 -> hiragana 'あ' (U+3042)
        let code: u16 = (0x24_u16 << 8) | 0x22;
        assert_eq!(jis0208_to_char(code), Some('あ'));
    }

    #[test]
    fn jis0208_hiragana_ka() {
        // Row 0x24, col 0x2b -> 'か' (U+304B)
        // (hiragana are not contiguous; verified via EUC-JP decode table)
        let code: u16 = (0x24_u16 << 8) | 0x2b;
        assert_eq!(jis0208_to_char(code), Some('か'));
    }

    #[test]
    fn jis0208_out_of_range_is_none() {
        // Row byte 0x00 is outside 0x21–0x7e
        assert_eq!(jis0208_to_char(0x0024), None);
        // Col byte 0x00
        assert_eq!(jis0208_to_char(0x2400), None);
        // Both outside
        assert_eq!(jis0208_to_char(0x0000), None);
    }

    // -----------------------------------------------------------------------
    // EtlFormat helpers
    // -----------------------------------------------------------------------

    #[test]
    fn etlformat_record_bytes() {
        assert_eq!(EtlFormat::M.record_bytes(), 2052);
        assert_eq!(EtlFormat::B8.record_bytes(), 512);
        assert_eq!(EtlFormat::B9.record_bytes(), 576);
        assert_eq!(EtlFormat::G8.record_bytes(), 8199);
        assert_eq!(EtlFormat::G9.record_bytes(), 8199);
    }

    #[test]
    fn etlformat_dummy_first_record() {
        assert!(EtlFormat::B8.has_dummy_first_record());
        assert!(EtlFormat::B9.has_dummy_first_record());
        assert!(!EtlFormat::M.has_dummy_first_record());
        assert!(!EtlFormat::G8.has_dummy_first_record());
        assert!(!EtlFormat::G9.has_dummy_first_record());
    }

    #[test]
    fn etlformat_from_str() {
        assert_eq!(EtlFormat::from_str("etl1"), Some(EtlFormat::M));
        assert_eq!(EtlFormat::from_str("ETL7"), Some(EtlFormat::M));
        assert_eq!(EtlFormat::from_str("etl8b"), Some(EtlFormat::B8));
        assert_eq!(EtlFormat::from_str("etl9b"), Some(EtlFormat::B9));
        assert_eq!(EtlFormat::from_str("etl8g"), Some(EtlFormat::G8));
        assert_eq!(EtlFormat::from_str("etl9g"), Some(EtlFormat::G9));
        assert_eq!(EtlFormat::from_str("etl2"), Some(EtlFormat::K));
        assert_eq!(EtlFormat::from_str("etl3"), Some(EtlFormat::C));
        assert_eq!(EtlFormat::from_str("bogus"), None);
    }

    // -----------------------------------------------------------------------
    // parse_mtype - synthetic record with known values
    // -----------------------------------------------------------------------

    fn make_mtype_record(jis_byte: u8, img_nibble_hi: u8, img_nibble_lo: u8) -> Vec<u8> {
        let mut rec = vec![0u8; 2052];
        // JIS X 0201 code at offset 6 (byte 7, 1-indexed)
        rec[6] = jis_byte;
        // First image byte at offset 32: high nibble = img_nibble_hi, low = img_nibble_lo
        rec[32] = (img_nibble_hi << 4) | (img_nibble_lo & 0x0f);
        rec
    }

    #[test]
    fn parse_mtype_jis_code_offset() {
        // Katakana 'ｱ': JIS X 0201 = 0xb1 -> U+FF71 in normal mode
        let rec = make_mtype_record(0xb1, 0, 0);
        let result = parse_mtype(&rec, None, false);
        assert_eq!(result.character, char::from_u32(0xFF71));
        assert_eq!(result.raw_code, 0xb1);
    }

    #[test]
    fn parse_mtype_image_offset() {
        // Place a known nibble pattern at offset 32 and verify it is read.
        // hi=0xf(15)->15*17=255, lo=0x0(0)->0
        let rec = make_mtype_record(0x41, 0xf, 0x0);
        let result = parse_mtype(&rec, None, false);
        assert_eq!(result.pixels[0], 255); // high nibble of byte at offset 32
        assert_eq!(result.pixels[1], 0); // low nibble
        assert_eq!(result.width, 64);
        assert_eq!(result.height, 63);
        assert_eq!(result.pixels.len(), 64 * 63);
    }

    #[test]
    fn parse_mtype_wrong_offset_would_be_zero() {
        // The old buggy code read the JIS code from offsets [4..5].
        // With our corrected record the bytes at [4..5] are 0x00 0x00 so if the
        // old offset were used the character would be None (code 0).
        // Here we verify the fix: offset 6 has our known value 0xb1 and the
        // character is decoded correctly.
        let mut rec = vec![0u8; 2052];
        rec[4] = 0x00; // would decode as None with old broken code
        rec[5] = 0x00;
        rec[6] = 0xb1; // correct JIS code
        let result = parse_mtype(&rec, None, false);
        assert_eq!(result.character, char::from_u32(0xFF71));
    }

    #[test]
    fn parse_mtype_normal_katakana_unchanged() {
        // 0xb1 in normal mode -> ｱ (U+FF71 halfwidth katakana A)
        let rec = make_mtype_record(0xb1, 0, 0);
        let result = parse_mtype(&rec, None, false);
        assert_eq!(result.character, Some('ｱ'));
    }

    #[test]
    fn parse_mtype_etl7_hiragana_mode() {
        // 0xb1 in ETL7/hiragana mode -> あ (U+3042) not ｱ (U+FF71)
        let rec = make_mtype_record(0xb1, 0, 0);
        let result = parse_mtype(&rec, None, true);
        assert_eq!(result.character, Some('あ'));
        // raw_code is unchanged - still reflects the byte in the file
        assert_eq!(result.raw_code, 0xb1);
    }

    #[test]
    fn parse_mtype_etl7_ascii_unchanged() {
        // ASCII range is identical in both modes
        let rec = make_mtype_record(0x41, 0, 0);
        let normal = parse_mtype(&rec, None, false);
        let hiragana = parse_mtype(&rec, None, true);
        assert_eq!(normal.character, hiragana.character);
        assert_eq!(normal.character, Some('A'));
    }

    // -----------------------------------------------------------------------
    // parse_8b - synthetic record
    // -----------------------------------------------------------------------

    fn make_8b_record(size: usize, jis_row: u8, jis_col: u8, first_img_byte: u8) -> Vec<u8> {
        let mut rec = vec![0u8; size];
        // JIS X 0208 code at offsets [2..4] big-endian
        rec[2] = jis_row;
        rec[3] = jis_col;
        // First image byte at offset 8
        rec[8] = first_img_byte;
        rec
    }

    #[test]
    fn parse_8b_etl8b_character_decode() {
        // JIS row=0x24, col=0x22 -> 'あ'
        let rec = make_8b_record(512, 0x24, 0x22, 0b1000_0000);
        let result = parse_8b(&rec, None);
        assert_eq!(result.character, Some('あ'));
        assert_eq!(result.raw_code, (0x24_u16 << 8) | 0x22);
    }

    #[test]
    fn parse_8b_image_offset() {
        // 0b10000000 at offset 8: MSB first -> first pixel = 1 -> 255
        let rec = make_8b_record(512, 0x24, 0x22, 0b1000_0000);
        let result = parse_8b(&rec, None);
        assert_eq!(result.pixels[0], 255);
        assert_eq!(result.pixels[1], 0);
        assert_eq!(result.pixels.len(), 64 * 63);
    }

    #[test]
    fn parse_8b_wrong_offset_would_miss_reading_field() {
        // The old buggy code read the image from offset 6 (skipping only the
        // 2-byte sheet number and 2-byte JIS code, but NOT the 4-byte reading
        // field). Here we put 0xff at offset 6 and 0b10000000 at offset 8.
        // With the fixed code offset 8 is used so pixel[0]=255.
        // If old offset 6 were used, the 0xff byte would be interpreted as
        // 1bpp all-ones giving pixel[0]=255 too - but pixel[4] would be
        // 0 (from offset 7 which is 0x00), whereas correct offset 8 gives
        // pixel[4] from the 5th bit of 0b10000000 = 0.
        let mut rec = vec![0u8; 512];
        rec[2] = 0x24;
        rec[3] = 0x22;
        rec[6] = 0xff; // old offset - would give all-255 for first 8 pixels
        rec[7] = 0x00;
        rec[8] = 0b1000_0000; // correct offset - first pixel=255, rest=0
        let result = parse_8b(&rec, None);
        // With correct offset 8: pixel[0]=255, pixel[1..8]=0
        assert_eq!(result.pixels[0], 255);
        assert_eq!(&result.pixels[1..8], &[0u8; 7]);
    }

    #[test]
    fn parse_8b_etl9b_size_accepted() {
        // ETL9B records are 576 bytes - parser should accept both 512 and 576
        let rec = make_8b_record(576, 0x24, 0x22, 0x00);
        let result = parse_8b(&rec, None);
        assert_eq!(result.character, Some('あ'));
    }

    // -----------------------------------------------------------------------
    // parse_8g - synthetic records for ETL8G and ETL9G
    // -----------------------------------------------------------------------

    fn make_8g_record(jis_row: u8, jis_col: u8, img_offset: usize, first_nibble: u8) -> Vec<u8> {
        let mut rec = vec![0u8; 8199];
        rec[2] = jis_row;
        rec[3] = jis_col;
        rec[img_offset] = first_nibble << 4; // high nibble
        rec
    }

    #[test]
    fn parse_8g_etl8g_jis_and_image() {
        // JIS row=0x24, col=0x22 -> 'あ'; high nibble = 0xf -> 255
        let rec = make_8g_record(0x24, 0x22, 60, 0xf);
        let result = parse_8g(&rec, 60, None);
        assert_eq!(result.character, Some('あ'));
        assert_eq!(result.pixels[0], 255);
        assert_eq!(result.width, 128);
        assert_eq!(result.height, 127);
    }

    #[test]
    fn parse_8g_etl9g_different_image_offset() {
        // ETL9G: image at offset 64, not 60.
        // Place a known value at offset 64 and verify it is picked up.
        let mut rec = vec![0u8; 8199];
        rec[2] = 0x24;
        rec[3] = 0x22;
        rec[60] = 0xff; // would give pixel=255 if wrong offset 60 were used
        rec[64] = 0b1111_0000_u8; // high nibble=0xf -> 255 at correct offset
        let result = parse_8g(&rec, 64, None);
        assert_eq!(result.pixels[0], 255); // from offset 64
        // Also verify: if we had wrongly used offset 60, pixel[0] would also be
        // 255 (since rec[60]=0xff). Let's test pixel[1] instead - at offset 60
        // rec[60]=0xff so lo nibble=0xf->255; at offset 64 rec[64]=0xf0 lo=0->0.
        assert_eq!(result.pixels[1], 0); // low nibble of rec[64]=0xf0 -> 0
    }

    #[test]
    fn parse_8g_old_euc_offset_was_wrong() {
        // The old code read jis_raw from record[4..12] and called euc_to_jis on [4] and [5].
        // With our fix the JIS code is at [2..4] directly.
        // Place a known JIS code at [2..4] and garbage at [4..6]:
        let mut rec = vec![0u8; 8199];
        rec[2] = 0x24; // JIS row for hiragana
        rec[3] = 0x22; // JIS col for 'あ'
        rec[4] = 0xaa; // garbage - old code would decode this as EUC high byte
        rec[5] = 0xbb; // garbage
        let result = parse_8g(&rec, 60, None);
        // With fixed code we get 'あ' from [2..4]
        assert_eq!(result.character, Some('あ'));
    }

    // -----------------------------------------------------------------------
    // resize functions
    // -----------------------------------------------------------------------

    #[test]
    fn resize_to_28_output_size() {
        let pixels = vec![128u8; 64 * 63];
        let out = resize_to_28(&pixels, 64, 63);
        assert_eq!(out.len(), 784);
    }

    #[test]
    fn resize_to_64_output_size() {
        let pixels = vec![200u8; 64 * 63];
        let out = resize_to_64(&pixels, 64, 63);
        assert_eq!(out.len(), 4096);
    }

    #[test]
    fn resize_preserves_uniform_image() {
        // A completely uniform gray image should stay uniform after resize.
        let pixels = vec![100u8; 128 * 127];
        let out28 = resize_to_28(&pixels, 128, 127);
        let out64 = resize_to_64(&pixels, 128, 127);
        // All pixels should remain 100 (bilinear of constant = constant)
        assert!(out28.iter().all(|&v| v == 100), "28x28 uniform failed");
        assert!(out64.iter().all(|&v| v == 100), "64x64 uniform failed");
    }

    // -----------------------------------------------------------------------
    // Native-resolution pixel counts (sanity check format geometry)
    // -----------------------------------------------------------------------

    #[test]
    fn native_res_pixel_counts() {
        // M-type / B-type: 64x63 = 4032
        assert_eq!(64usize * 63, 4032);
        // G-type: 128x127 = 16256
        assert_eq!(128usize * 127, 16256);
        // C-type: 72x76 = 5472
        assert_eq!(72usize * 76, 5472);
        // K-type: 60x60 = 3600
        assert_eq!(60usize * 60, 3600);
    }

    // -----------------------------------------------------------------------
    // unpack_6bpp_to_8bit
    // -----------------------------------------------------------------------

    #[test]
    fn unpack_6bpp_four_pixels_one_group() {
        // 3 bytes -> 4 6-bit pixels
        // b0=0b11111100 b1=0b00001111 b2=0b00111111
        // p0 = b0>>2        = 0b111111 = 63 -> 255
        // p1 = (b0&3)<<4|(b1>>4) = 0b000000 = 0  -> 0
        // p2 = (b1&0xf)<<2|(b2>>6) = 0b111100 = 60 -> (60*255/63)=242
        // p3 = b2&0x3f      = 0b111111 = 63 -> 255
        let src = [0b1111_1100_u8, 0b0000_1111, 0b0011_1111];
        let out = unpack_6bpp_to_8bit(&src, 4);
        assert_eq!(out[0], 255);
        assert_eq!(out[1], 0);
        assert_eq!(out[2], (60u16 * 255 / 63) as u8);
        assert_eq!(out[3], 255);
    }

    #[test]
    fn unpack_6bpp_truncates_to_n() {
        // Only ask for 2 pixels out of a 3-byte group
        let src = [0xff_u8, 0x00, 0xff];
        let out = unpack_6bpp_to_8bit(&src, 2);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn unpack_6bpp_full_image_length() {
        // K-type image: 2700 bytes -> 3600 pixels
        let src = vec![0u8; 2700];
        let out = unpack_6bpp_to_8bit(&src, 3600);
        assert_eq!(out.len(), 3600);
    }

    // -----------------------------------------------------------------------
    // parse_ctype - synthetic record
    // -----------------------------------------------------------------------

    fn make_ctype_record(jis_byte: u8, img_nibble_hi: u8, img_nibble_lo: u8) -> Vec<u8> {
        let mut rec = vec![0u8; 2952];
        // JIS byte at offset 9 (= top 8 bits of 6-bit chars 12-17, bit 72)
        rec[9] = jis_byte;
        // First image byte at offset 216
        rec[216] = (img_nibble_hi << 4) | (img_nibble_lo & 0x0f);
        rec
    }

    #[test]
    fn parse_ctype_jis_byte_offset() {
        // Katakana 'ｱ': JIS X 0201 = 0xb1 -> U+FF71 in normal mode
        let rec = make_ctype_record(0xb1, 0, 0);
        let result = parse_ctype(&rec, None, false);
        assert_eq!(result.character, char::from_u32(0xFF71));
        assert_eq!(result.raw_code, 0xb1);
        assert_eq!(result.width, 72);
        assert_eq!(result.height, 76);
        assert_eq!(result.pixels.len(), 72 * 76);
    }

    #[test]
    fn parse_ctype_image_offset() {
        // hi nibble 0xf=15 -> 15*17=255, lo nibble 0x0=0 -> 0
        let rec = make_ctype_record(0x41, 0xf, 0x0);
        let result = parse_ctype(&rec, None, false);
        assert_eq!(result.pixels[0], 255);
        assert_eq!(result.pixels[1], 0);
    }

    #[test]
    fn parse_ctype_ascii_char() {
        // ASCII 'A' = 0x41 - same in both modes
        let rec = make_ctype_record(0x41, 0, 0);
        let result = parse_ctype(&rec, None, false);
        assert_eq!(result.character, Some('A'));
    }

    #[test]
    fn parse_ctype_normal_katakana_unchanged() {
        // 0xb1 in normal mode -> ｱ (U+FF71 halfwidth katakana A)
        let rec = make_ctype_record(0xb1, 0, 0);
        let result = parse_ctype(&rec, None, false);
        assert_eq!(result.character, Some('ｱ'));
    }

    #[test]
    fn parse_ctype_etl4_hiragana_mode() {
        // 0xb1 in ETL4/hiragana mode -> あ (U+3042) not ｱ (U+FF71)
        let rec = make_ctype_record(0xb1, 0, 0);
        let result = parse_ctype(&rec, None, true);
        assert_eq!(result.character, Some('あ'));
        // raw_code is unchanged - it still reflects the byte in the file
        assert_eq!(result.raw_code, 0xb1);
    }

    #[test]
    fn parse_ctype_etl4_ascii_unchanged() {
        // ASCII range is identical in both modes
        let rec = make_ctype_record(0x41, 0, 0);
        let normal = parse_ctype(&rec, None, false);
        let hiragana = parse_ctype(&rec, None, true);
        assert_eq!(normal.character, hiragana.character);
        assert_eq!(normal.character, Some('A'));
    }

    // -----------------------------------------------------------------------
    // jis0201_kana_as_hiragana - ETL4 katakana->hiragana remap
    // -----------------------------------------------------------------------

    #[test]
    fn jis0201_kana_as_hiragana_spot_checks() {
        // Vowels
        assert_eq!(jis0201_kana_as_hiragana(0xb1), Some('あ')); // ｱ -> あ
        assert_eq!(jis0201_kana_as_hiragana(0xb3), Some('う')); // ｳ -> う
        // Small kana
        assert_eq!(jis0201_kana_as_hiragana(0xa7), Some('ぁ')); // ｧ -> ぁ small a
        assert_eq!(jis0201_kana_as_hiragana(0xaf), Some('っ')); // ｯ -> っ small tsu
        assert_eq!(jis0201_kana_as_hiragana(0xac), Some('ゃ')); // ｬ -> ゃ small ya
        // wo / n / wa
        assert_eq!(jis0201_kana_as_hiragana(0xa6), Some('を')); // ｦ -> を
        assert_eq!(jis0201_kana_as_hiragana(0xdd), Some('ん')); // ﾝ -> ん
        assert_eq!(jis0201_kana_as_hiragana(0xdc), Some('わ')); // ﾜ -> わ
        // Codes with no hiragana equivalent -> None
        assert_eq!(jis0201_kana_as_hiragana(0xa1), None); // ｡ punctuation
        assert_eq!(jis0201_kana_as_hiragana(0xa5), None); // ･ middle dot
        assert_eq!(jis0201_kana_as_hiragana(0xb0), None); // ｰ prolonged sound mark
        assert_eq!(jis0201_kana_as_hiragana(0xde), None); // ﾞ voiced sound mark
        assert_eq!(jis0201_kana_as_hiragana(0xdf), None); // ﾟ semi-voiced sound mark
        // ASCII pass-through
        assert_eq!(jis0201_kana_as_hiragana(0x41), Some('A'));
        assert_eq!(jis0201_kana_as_hiragana(0x5c), Some('¥')); // JIS special
    }

    // -----------------------------------------------------------------------
    // parse_ktype - synthetic record
    // -----------------------------------------------------------------------

    fn make_ktype_record(col: u8, row: u8, first_img_byte: u8) -> Vec<u8> {
        let mut rec = vec![0u8; 2745];
        // CO-59 code: col (6 bits) packed into top 6 bits of byte 21,
        // row (6 bits) packed into bottom 2 bits of byte 21 and top 4 bits of byte 22.
        //   byte 21 = col << 2 | row >> 4
        //   byte 22 = (row & 0x0f) << 4
        rec[21] = (col << 2) | (row >> 4);
        rec[22] = (row & 0x0f) << 4;
        // First image byte at offset 45
        rec[45] = first_img_byte;
        rec
    }

    #[test]
    fn parse_ktype_raw_code() {
        // col=4, row=4 -> raw_code = (4<<6)|4 = 260; maps to '上' (U+4E0A)
        let rec = make_ktype_record(4, 4, 0);
        let result = parse_ktype(&rec, None);
        assert_eq!(result.raw_code, (4u16 << 6) | 4);
        assert_eq!(result.character, Some('上'));
        assert_eq!(result.width, 60);
        assert_eq!(result.height, 60);
        assert_eq!(result.pixels.len(), 60 * 60);
    }

    #[test]
    fn parse_ktype_image_offset() {
        // 0b11111100 = first 6-bit pixel = 63 -> 255
        let rec = make_ktype_record(0, 0, 0b1111_1100);
        let result = parse_ktype(&rec, None);
        assert_eq!(result.pixels[0], 255);
    }

    #[test]
    fn parse_ktype_co59_code_boundary() {
        // col=59, row=59 -> raw_code = (59<<6)|59 = 3835; maps to '亀' (U+4E80)
        let rec = make_ktype_record(59, 59, 0);
        let result = parse_ktype(&rec, None);
        assert_eq!(result.raw_code, (59u16 << 6) | 59);
        assert_eq!(result.character, Some('亀'));
    }

    // -----------------------------------------------------------------------
    // write_npz round-trip - write a tiny array and read it back
    // -----------------------------------------------------------------------

    #[test]
    fn write_npz_round_trip_u8() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.npz");
        let path_str = path.to_str().unwrap();

        let data: Vec<u8> = (0..6).collect();
        write_npz(path_str, &data, &[2, 3], "|u1").expect("write_npz");

        // Open the zip and read arr_0.npy back
        let file = fs::File::open(path_str).expect("open");
        let mut zip = zip::ZipArchive::new(file).expect("zip");
        let mut entry = zip.by_name("arr_0.npy").expect("arr_0.npy");
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut raw).expect("read");

        // Magic bytes
        assert_eq!(&raw[..6], b"\x93NUMPY");
        // Major version = 1
        assert_eq!(raw[6], 1);

        // Data must be present at the end - last 6 bytes
        assert_eq!(&raw[raw.len() - 6..], &[0u8, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn write_npz_round_trip_u16() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test16.npz");
        let path_str = path.to_str().unwrap();

        // 3 u16 values: 0, 300, 1000
        let values: Vec<u16> = vec![0, 300, 1000];
        let data: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        write_npz(path_str, &data, &[3], "<u2").expect("write_npz u16");

        let file = fs::File::open(path_str).expect("open");
        let mut zip = zip::ZipArchive::new(file).expect("zip");
        let mut entry = zip.by_name("arr_0.npy").expect("arr_0.npy");
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut raw).expect("read");

        // Last 6 bytes: 0x0000 0x2c01 0xe803 (LE u16 values)
        let tail = &raw[raw.len() - 6..];
        assert_eq!(u16::from_le_bytes([tail[0], tail[1]]), 0u16);
        assert_eq!(u16::from_le_bytes([tail[2], tail[3]]), 300u16);
        assert_eq!(u16::from_le_bytes([tail[4], tail[5]]), 1000u16);
    }

    // -----------------------------------------------------------------------
    // label_map_from_records - auto-builds sorted map from record stream
    // -----------------------------------------------------------------------

    #[test]
    fn label_map_from_records_sorted() {
        // Build two synthetic records with known characters
        let rec_a = EtlRecord {
            character: Some('あ'),
            raw_code: 0,
            pixels: vec![0; 64 * 63],
            width: 64,
            height: 63,
            source_file: None,
        };
        let rec_b = EtlRecord {
            character: Some('A'),
            raw_code: 0,
            pixels: vec![0; 64 * 63],
            width: 64,
            height: 63,
            source_file: None,
        };
        let no_equiv = HashMap::new();
        let map = label_map_from_records(&[rec_a, rec_b], &no_equiv);
        // Sorted by Unicode code point: 'A' (U+0041) < 'あ' (U+3042)
        assert_eq!(map[&'A'], 0);
        assert_eq!(map[&'あ'], 1);
    }

    #[test]
    fn label_map_from_records_deduplicates() {
        let rec = EtlRecord {
            character: Some('ｱ'),
            raw_code: 0,
            pixels: vec![0; 64 * 63],
            width: 64,
            height: 63,
            source_file: None,
        };
        let no_equiv = HashMap::new();
        let map = label_map_from_records(&[rec.clone(), rec], &no_equiv);
        assert_eq!(map.len(), 1);
        assert_eq!(map[&'ｱ'], 0);
    }

    #[test]
    fn label_map_from_records_merges_halfwidth_via_equiv() {
        // With the halfwidth equiv map, ｱ (FF71) should collapse to ア (30A2).
        let rec = EtlRecord {
            character: Some('\u{FF71}'),
            raw_code: 0,
            pixels: vec![0; 64 * 63],
            width: 64,
            height: 63,
            source_file: None,
        };
        let equiv = crate::kana_merging::halfwidth_equiv();
        let map = label_map_from_records(&[rec], &equiv);
        assert_eq!(map.len(), 1);
        assert!(
            map.contains_key(&'ア'),
            "map should key on canonical ア not halfwidth ｱ"
        );
        assert!(
            !map.contains_key(&'ｱ'),
            "halfwidth key should not appear in map"
        );
    }

    // merge_halfwidth tests live in kana_merging.rs

    // -----------------------------------------------------------------------
    // dummy-record skip: read_etl_file skips record 0 for B8/B9
    // -----------------------------------------------------------------------

    #[test]
    fn read_etl_file_skips_dummy_b8() {
        // Build a minimal ETL8B file: 2 records x 512 bytes.
        // Record 0 (dummy): JIS code 0x0000 -> None character.
        // Record 1 (real):  JIS row=0x24 col=0x22 -> 'あ'.
        let mut file_bytes = vec![0u8; 512 * 2];
        // record 1 JIS code at offsets 512+2 and 512+3
        file_bytes[512 + 2] = 0x24;
        file_bytes[512 + 3] = 0x22;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ETL8B_test");
        fs::write(&path, &file_bytes).expect("write test file");

        let records = read_etl_file(path.to_str().unwrap(), EtlFormat::B8).expect("read_etl_file");

        // Should have exactly 1 record (dummy record 0 was skipped)
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].character, Some('あ'));
    }

    #[test]
    fn read_etl_file_no_dummy_for_mtype() {
        // M-type: no dummy record. 2 records x 2052 bytes.
        // Both get JIS code at offset 6.
        let mut file_bytes = vec![0u8; 2052 * 2];
        file_bytes[6] = 0x41; // record 0: 'A'
        file_bytes[2052 + 6] = 0x42; // record 1: 'B'

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ETL1_test");
        fs::write(&path, &file_bytes).expect("write test file");

        let records = read_etl_file(path.to_str().unwrap(), EtlFormat::M).expect("read_etl_file");

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].character, Some('A'));
        assert_eq!(records[1].character, Some('B'));
    }

    // -----------------------------------------------------------------------
    // read_etl_dir: non-data files (e.g. *INFO text files) must be skipped
    // -----------------------------------------------------------------------

    #[test]
    fn read_etl_dir_skips_non_multiple_files() {
        // Simulates the ETL8B directory layout:
        //   ETL8B2C1  - valid data file (2 x 512 = 1024 bytes)
        //   ETL8INFO  - text file (14723 bytes, NOT a multiple of 512)
        //
        // Only ETL8B2C1 should be parsed. ETL8INFO contains text whose bytes
        // at [2..4] of some 512-byte slice accidentally decode to a valid JIS
        // codepoint, producing a phantom character if the file is not rejected.
        let dir = tempfile::tempdir().expect("tempdir");

        // Valid data file: 2 records, record 1 (after dummy skip) -> 'あ'
        let mut data_bytes = vec![0u8; 512 * 2];
        data_bytes[512 + 2] = 0x24; // JIS row for hiragana
        data_bytes[512 + 3] = 0x22; // JIS col -> 'あ'
        fs::write(dir.path().join("ETL8B2C1"), &data_bytes).expect("write data");

        // Non-data INFO file: 14723 bytes - not a multiple of 512.
        // Fill with non-zero bytes so that if it were parsed some would
        // accidentally land in the valid JIS range 0x21–0x7e.
        let info_bytes = vec![0x5fu8; 14723];
        fs::write(dir.path().join("ETL8INFO"), &info_bytes).expect("write info");

        let records =
            read_etl_dir(dir.path().to_str().unwrap(), EtlFormat::B8, &[]).expect("read_etl_dir");

        // Only the one real record from the data file should appear.
        assert_eq!(
            records.len(),
            1,
            "INFO file must be rejected, got {records:?}"
        );
        assert_eq!(records[0].character, Some('あ'));
    }

    /// Verify the unicode_names2 crate returns names that contain "letter small"
    /// for all small kana, so the filter_reason() function correctly excludes them.
    #[test]
    fn filter_reason_small_kana_excluded_by_letter_small() {
        let small_kana = [
            '\u{30A1}', // ァ KATAKANA LETTER SMALL A
            '\u{30A3}', // ィ KATAKANA LETTER SMALL I
            '\u{30A5}', // ゥ KATAKANA LETTER SMALL U
            '\u{30A7}', // ェ KATAKANA LETTER SMALL E
            '\u{30A9}', // ォ KATAKANA LETTER SMALL O
            '\u{30E3}', // ャ KATAKANA LETTER SMALL YA
            '\u{30E5}', // ュ KATAKANA LETTER SMALL YU
            '\u{30E7}', // ョ KATAKANA LETTER SMALL YO
            '\u{30C3}', // ッ KATAKANA LETTER SMALL TU
            '\u{30EE}', // ヮ KATAKANA LETTER SMALL WA
        ];
        let filter_names = vec!["letter small".to_string()];
        for ch in small_kana {
            let name = unicode_names2::name(ch)
                .map(|n| n.to_string())
                .unwrap_or_else(|| "NONE".to_string());
            let reason = filter_reason(ch, &[], &filter_names);
            assert!(
                reason.is_some(),
                "U+{:04X} {:?} (name={:?}) should be filtered by 'letter small' but was not",
                ch as u32,
                ch,
                name
            );
        }
    }

    /// Simulate the exact label_map build to verify small kana do NOT appear.
    ///
    /// This test simulates what label_map_from_records does with ETL records that
    /// include both small kana chars (which should be filtered) and large kana chars.
    /// The label_map should only contain non-small, non-halfwidth chars.
    #[test]
    fn label_map_from_records_excludes_small_kana() {
        use crate::kana_merging::halfwidth_equiv;

        let filter_names = vec!["letter small".to_string(), "halfwidth".to_string()];
        let equiv = halfwidth_equiv();

        // Simulate ETL records: ヤ (30E4), ャ (30E3, should be filtered), ﾔ (FF94, halfwidth, should be filtered)
        let records: Vec<EtlRecord> = vec![
            EtlRecord {
                character: Some('\u{30E4}'), // ヤ - large Ya, should survive
                raw_code: 0x2564,
                pixels: vec![0u8; 64 * 63],
                width: 64,
                height: 63,
                source_file: None,
            },
            EtlRecord {
                character: Some('\u{30E3}'), // ャ - small Ya, should be filtered
                raw_code: 0x2563,
                pixels: vec![0u8; 64 * 63],
                width: 64,
                height: 63,
                source_file: None,
            },
            EtlRecord {
                character: Some('\u{FF94}'), // ﾔ - halfwidth Ya, should be filtered
                raw_code: 0xD4,
                pixels: vec![0u8; 64 * 63],
                width: 64,
                height: 63,
                source_file: None,
            },
            EtlRecord {
                character: Some('\u{30C4}'), // ツ - large Tu, should survive
                raw_code: 0x2544,
                pixels: vec![0u8; 64 * 63],
                width: 64,
                height: 63,
                source_file: None,
            },
            EtlRecord {
                character: Some('\u{30C3}'), // ッ - small Tu, should be filtered
                raw_code: 0x2543,
                pixels: vec![0u8; 64 * 63],
                width: 64,
                height: 63,
                source_file: None,
            },
            EtlRecord {
                character: Some('\u{FF82}'), // ﾂ - halfwidth Tu, should be filtered
                raw_code: 0xC2,
                pixels: vec![0u8; 64 * 63],
                width: 64,
                height: 63,
                source_file: None,
            },
        ];

        // Apply filter (simulating write_merged_etlcdb's filter step)
        let filtered: Vec<EtlRecord> = records
            .iter()
            .filter(|r| {
                r.character
                    .map(|ch| filter_reason(ch, &[], &filter_names).is_none())
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        let label_map = label_map_from_records(&filtered, &equiv);

        assert!(
            !label_map.contains_key(&'\u{30E3}'),
            "ャ (small ya) must not be in label_map"
        );
        assert!(
            !label_map.contains_key(&'\u{30C3}'),
            "ッ (small tu) must not be in label_map"
        );
        assert!(
            label_map.contains_key(&'\u{30E4}'),
            "ヤ (ya) must be in label_map"
        );
        assert!(
            label_map.contains_key(&'\u{30C4}'),
            "ツ (tu) must be in label_map"
        );
        // Halfwidth forms should NOT appear (they were filtered AND equiv maps them to small)
        assert!(
            !label_map.contains_key(&'\u{FF94}'),
            "ﾔ (halfwidth ya) must not be in label_map"
        );
        assert!(
            !label_map.contains_key(&'\u{FF82}'),
            "ﾂ (halfwidth tu) must not be in label_map"
        );
    }

    /// Regression test for the filter-bypass bug in the output loop:
    ///
    /// Before the fix, `write_merged_etlcdb` (and `convert_etlcdb`) would
    /// iterate the raw unfiltered `batch.records` in the write step. Any
    /// record whose original char was filtered-out but whose
    /// `equiv_char(ch, equiv)` canonical WAS in the label_map (from a
    /// different, non-filtered record that shared the same canonical) would
    /// have its image silently written into the wrong class.
    ///
    /// Concrete example: halfwidth ﾞ (U+FF9E, "HALFWIDTH KATAKANA VOICED
    /// SOUND MARK") is filtered by the "sound mark" name pattern, but its
    /// canonical U+309B is in the label_map if any non-filtered record maps
    /// there. With the bug, U+FF9E images leaked into U+309B's class.
    ///
    /// This test verifies that the filter is enforced at the output step:
    /// the filtered-out record must NOT contribute an image.
    #[test]
    fn output_loop_filter_not_bypassed_by_equiv_canonical() {
        use crate::kana_merging::halfwidth_equiv;

        // Filter "sound mark" to exclude U+FF9E (halfwidth voiced sound mark).
        let filter_names = vec!["sound mark".to_string()];
        let equiv = halfwidth_equiv();

        // Two records: one that is filtered (U+FF9E) and one that is not
        // (U+30A2 ア - fullwidth katakana A, passes any kana filter).
        let filtered_rec = EtlRecord {
            character: Some('\u{FF9E}'), // ﾞ halfwidth voiced sound mark - should be filtered
            raw_code: 0xDE,
            pixels: vec![1u8; 64 * 63], // distinct pixel value to detect leakage
            width: 64,
            height: 63,
            source_file: None,
        };
        let kept_rec = EtlRecord {
            character: Some('\u{30A2}'), // ア fullwidth katakana A - not filtered
            raw_code: 0x2522,
            pixels: vec![0u8; 64 * 63],
            width: 64,
            height: 63,
            source_file: None,
        };

        // Simulate the filter -> label_map build step (the fix means
        // filter_reason is also checked in the output loop, so confirm that
        // the filtered record's canonical (U+309B) does NOT end up in the
        // label_map in the first place when filtered out).
        let all_records = vec![filtered_rec.clone(), kept_rec.clone()];
        let kept: Vec<EtlRecord> = all_records
            .iter()
            .filter(|r| {
                r.character
                    .map(|ch| filter_reason(ch, &[], &filter_names).is_none())
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        let label_map = label_map_from_records(&kept, &equiv);

        // Only ア should be in the label_map.
        assert!(
            label_map.contains_key(&'\u{30A2}'),
            "ア must be in label_map"
        );
        // U+309B (fullwidth voiced sound mark) is U+FF9E's canonical - it must
        // NOT be in the label_map because U+FF9E was filtered.
        assert!(
            !label_map.contains_key(&'\u{309B}'),
            "U+309B (voiced sound mark canonical) must not be in label_map - \
             its only source (U+FF9E) was filtered out"
        );

        // Now simulate the output loop with the filter guard in place.
        // Verify that the filtered record (U+FF9E) is rejected.
        let mut written_chars: Vec<char> = Vec::new();
        for rec in &all_records {
            let Some(ch) = rec.character else { continue };
            // This is the guard that was missing before the fix:
            if filter_reason(ch, &[], &filter_names).is_some() {
                continue;
            }
            let canonical = crate::kana_merging::equiv_char(ch, &equiv);
            if label_map.contains_key(&canonical) {
                written_chars.push(canonical);
            }
        }
        assert_eq!(
            written_chars,
            vec!['\u{30A2}'],
            "only ア should be written; filtered U+FF9E must not leak into output"
        );
    }

    /// Verify unicode_names2 returns the expected names for small and large katakana
    /// ya/yu/yo/tu so the filter can distinguish them.
    #[test]
    fn unicode_names2_small_kana_names() {
        let cases = [
            ('\u{30E3}', "KATAKANA LETTER SMALL YA"), // ャ
            ('\u{30E4}', "KATAKANA LETTER YA"),       // ヤ
            ('\u{30E5}', "KATAKANA LETTER SMALL YU"), // ュ
            ('\u{30E6}', "KATAKANA LETTER YU"),       // ユ
            ('\u{30E7}', "KATAKANA LETTER SMALL YO"), // ョ
            ('\u{30E8}', "KATAKANA LETTER YO"),       // ヨ
            ('\u{30C3}', "KATAKANA LETTER SMALL TU"), // ッ
            ('\u{30C4}', "KATAKANA LETTER TU"),       // ツ
        ];
        for (ch, expected_name) in cases {
            let got = unicode_names2::name(ch)
                .map(|n| n.to_string())
                .unwrap_or_else(|| "NONE".to_string());
            assert_eq!(
                got, expected_name,
                "U+{:04X} {:?}: expected name {:?}, got {:?}",
                ch as u32, ch, expected_name, got
            );
        }
    }
}
