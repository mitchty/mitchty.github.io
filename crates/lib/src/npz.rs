//! Shared code for reading and writing NumPy `.npz` / `.npy` files in rust
//!
//! # NPY v1 wire format (only what I need so far)
//!
//! ```text
//! magic    6 bytes  \x93NUMPY
//! major    1 byte   must be 1
//! minor    1 byte   0 or 1 for v1.x
//! hlen     2 bytes  little-endian u16: length of the header dict that follows
//! header   hlen bytes  Python-dict-style ASCII string, padded to 64-byte boundary
//! data     ...       raw array bytes, C-order (row-major)
//! ```
//!
//! `.npz` files are just zip archives where each entry is one `.npy` array.
//! We always write and expect to read a single entry named `arr_0.npy`.

#[cfg(not(target_arch = "wasm32"))]
use std::io;

/// A parsed NPY v1 header data.
#[derive(Debug, Clone)]
pub struct NpyHeader {
    /// NumPy dtype string, e.g. `"|u1"`, `"<u2"`, `"<f4"`.
    pub dtype: String,
    /// Array shape, e.g. `[N, H, W]` for images or `[N]` for labels.
    pub shape: Vec<usize>,
    /// Byte offset at which the raw array data begins.
    pub data_offset: usize,
}

/// Parse a NumPy v1.x NPY file from raw bytes.
///
/// Returns `Err(String)` with a human-readable message (including `path`) on
/// any format violation so callers can surface it directly in UI or logs.
pub fn parse_npy_header(raw: &[u8], path: &str) -> Result<NpyHeader, String> {
    if raw.len() < 10 {
        return Err(format!(
            "{path}: too short to be a valid NPY file ({} bytes)",
            raw.len()
        ));
    }
    if &raw[..6] != b"\x93NUMPY" {
        return Err(format!("{path}: not a valid NPY file (bad magic bytes)"));
    }
    let major = raw[6];
    if major != 1 {
        return Err(format!(
            "{path}: only NPY v1.x is supported, got major version {major} instead"
        ));
    }
    // minor version (raw[7]) is 0 or 1 for v1.x; doesn't affect data layout.

    let hlen = u16::from_le_bytes([raw[8], raw[9]]) as usize;
    let data_offset = 10 + hlen;
    if raw.len() < data_offset {
        return Err(format!(
            "{path}: truncated NPY header claims to be {hlen} header bytes, but file is only {} bytes",
            raw.len()
        ));
    }

    let header = std::str::from_utf8(&raw[10..data_offset])
        .map_err(|_| format!("{path}: NPY header is not valid UTF-8"))?;

    let dtype = npy_field(header, "descr")
        .ok_or_else(|| format!("{path}: NPY header missing 'descr' field"))?;

    let shape_str = npy_field(header, "shape")
        .ok_or_else(|| format!("{path}: NPY header missing 'shape' field"))?;

    let shape = npy_shape(&shape_str)
        .ok_or_else(|| format!("{path}: cannot parse shape tuple '{shape_str}'"))?;

    Ok(NpyHeader {
        dtype,
        shape,
        data_offset,
    })
}

/// Extract the value for `key` from a Python-dict-style NPY header string.
///
/// Handles the three value forms that appear in practice that I've seen so far:
/// - tuple: `'shape': (N, H, W)`  returns `"(N, H, W)"`
/// - quoted string: `'descr': '|u1'` returns `"|u1"`
/// - bare word: `'fortran_order': False` returns `"False"`
pub fn npy_field(header: &str, key: &str) -> Option<String> {
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

/// Parse a Python tuple like `"(270912,)"` or `"(N, H, W)"` to `Vec<usize>`.
pub fn npy_shape(s: &str) -> Option<Vec<usize>> {
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

/// Decode a flat byte buffer of labels into `Vec<u32>`.
///
/// Supports `"|u1"` (one byte per label, values 0–255) and `"<u2"` or
/// `"|u2"` (little-endian u16 per label, values 0–65535).
///
/// Any other dtype string falls through to the `|u1` path for now.
pub fn read_labels(data: &[u8], n: usize, dtype: &str) -> Vec<u32> {
    match dtype {
        "<u2" | "|u2" => (0..n)
            .map(|i| u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]) as u32)
            .collect(),
        _ => data[..n].iter().map(|&b| b as u32).collect(),
    }
}

/// Open an `.npz` file and return the raw bytes of its first entry.
///
/// If the archive has no entries or the entry cannot be read, returns
/// `Err(String)` with a human-readable message of the Err value.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_npz_first_entry(path: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("cannot open npz '{path}': {e}"))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("'{path}' is not a valid zip/npz archive: {e}"))?;

    let entry_name = zip
        .by_index(0)
        .map(|e| e.name().to_owned())
        .map_err(|e| format!("'{path}': archive has no entries: {e}"))?;

    let mut entry = zip
        .by_name(&entry_name)
        .map_err(|e| format!("'{path}': cannot open entry '{entry_name}': {e}"))?;

    let mut raw = Vec::new();
    entry
        .read_to_end(&mut raw)
        .map_err(|e| format!("'{path}': error reading entry '{entry_name}': {e}"))?;
    Ok(raw)
}

/// Write a single NumPy v1 array into a deflate-compressed `.npz` file.
///
/// The array is stored as the sole entry `arr_0.npy` inside the zip.
/// `dtype` must be a valid numpy dtype string: `"|u1"` (uint8) or `"<u2"`
/// (little-endian uint16). `shape` is e.g. `[N, H, W]` for images or
/// `[N]` for labels.
///
/// The NPY header is padded to a multiple of 64 bytes as required by the spec.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_npz(path: &str, data: &[u8], shape: &[usize], dtype: &str) -> io::Result<()> {
    use std::io::Write;

    let shape_str = shape
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let header_dict =
        format!("{{'descr': '{dtype}', 'fortran_order': False, 'shape': ({shape_str},), }}");

    // Header must be padded so that 10 + hlen is a multiple of 64 bytes to be valid
    let prefix_len = 10usize;
    let raw_len = header_dict.len() + 1; // +1 for the '\n' terminator
    let padded_total = (prefix_len + raw_len).div_ceil(64) * 64;
    let pad_len = padded_total - prefix_len - raw_len;
    let header_len = (raw_len + pad_len) as u16;

    let mut npy: Vec<u8> = Vec::new();
    npy.extend_from_slice(b"\x93NUMPY");
    npy.push(1); // major
    npy.push(0); // minor
    npy.extend_from_slice(&header_len.to_le_bytes());
    npy.extend_from_slice(header_dict.as_bytes());
    npy.extend(std::iter::repeat_n(b' ', pad_len));
    npy.push(b'\n');
    npy.extend_from_slice(data);

    // ZIP32 entries are limited to 4 GiB. The NPY header is tiny; the data
    // buffer is what can be large, so use its size as the heuristic. Enable
    // ZIP64 whenever the uncompressed entry would exceed the default 32-bit
    // limit for chungus datasets.
    const ZIP32_LIMIT: u64 = u32::MAX as u64; // 4 GiB − 1
    let needs_zip64 = npy.len() as u64 > ZIP32_LIMIT;

    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(needs_zip64);
    zip.start_file("arr_0.npy", options)?;
    zip.write_all(&npy)?;
    zip.finish()?;
    Ok(())
}
