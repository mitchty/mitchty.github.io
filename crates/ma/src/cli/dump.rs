//! `ma dump` - inspect individual ETL characters by Unicode codepoint.
//!
//! Loads one or more ETL datasets (same root dir / format tokens as
//! `ma convert`), finds every record matching the requested character(s),
//! prints a terminal ASCII-art preview for a quick sanity check, and writes
//! a contact-sheet PNG so you can eyeball all the writing samples at once.
//!
//! # Usage
//!
//! ```text
//! ma dump --input ~/src/models/source/etlcdb --char 漑
//! ma dump --input ~/src/models/source/etlcdb --char 漑 --char あ --out /tmp/dump
//! ma dump --input ~/src/models/source/etlcdb --char 漑 --cols 10 --out /tmp
//! ```

use std::fs;

use clap::Args;
use image::{GrayImage, ImageBuffer, Luma};

use crate::etl::{EtlBatch, EtlFormat, EtlRecord, read_etl_dir, read_etl_file, read_etlcdb_dir};

// ─── CLI args ────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct DumpArgs {
    /// ETLcdb root directory (containing ETL1…ETL9G sub-dirs) when using
    /// --format etlcdb, or path to a single ETL file / directory for
    /// single-dataset formats.
    #[arg(short, long)]
    input: String,

    /// Dataset format - same tokens as `ma convert --format`.
    ///
    ///   etlcdb   (default) all supported datasets under --input root
    ///   etl1 / etl6 / etl7 / etl8b / etl9b / etl8g / etl9g   single family
    #[arg(short, long, default_value = "etlcdb")]
    format: String,

    /// Unicode character(s) to inspect. Repeat for multiple characters:
    ///
    ///   --char 漑  --char あ  --char ア
    ///
    /// Each flag value must be exactly one Unicode scalar.
    #[arg(long = "char", value_name = "CHAR")]
    chars: Vec<String>,

    /// Directory to write contact-sheet PNGs (created if absent).
    #[arg(short, long, default_value = ".")]
    out: String,

    /// Number of sample columns in the contact-sheet PNG.
    #[arg(long, default_value_t = 8)]
    cols: u32,

    /// Print detailed debugging information for each character.
    ///
    /// For K-type (ETL2) data, this prints:
    ///   - Raw CO-59 code (col, row) extracted from bytes 21-22
    ///   - Raw bytes at positions 21-22 (hex)
    ///   - The mapped Unicode character
    ///   - Raw code value (12-bit)
    ///
    /// This helps debug off-by-one errors in the parser or input data.
    #[arg(short, long)]
    verbose: bool,
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn run(args: DumpArgs) {
    crate::cli::init_tracing();

    // Parse requested characters up-front so we can bail early on bad input.
    let targets: Vec<char> = args
        .chars
        .iter()
        .filter_map(|s| {
            let mut cs = s.chars();
            let ch = cs.next()?;
            if cs.next().is_some() {
                eprintln!("--char {s:?}: must be a single Unicode character, skipping");
                return None;
            }
            Some(ch)
        })
        .collect();

    if targets.is_empty() {
        eprintln!("no valid --char values supplied; nothing to do");
        std::process::exit(1);
    }

    fs::create_dir_all(&args.out).unwrap_or_else(|e| {
        eprintln!("cannot create output dir {}: {e}", args.out);
        std::process::exit(1);
    });

    // ── Load records ────────────────────────────────────────────────────────
    let batches: Vec<EtlBatch> = if args.format.to_lowercase() == "etlcdb" {
        read_etlcdb_dir(&args.input, &[]).unwrap_or_else(|e| {
            eprintln!("failed to read ETLcdb from {}: {e}", args.input);
            std::process::exit(1);
        })
    } else {
        let fmt = EtlFormat::from_str(&args.format).unwrap_or_else(|| {
            eprintln!(
                "unknown format {:?}; valid: etl1 etl6 etl7 etl8b etl9b etl8g etl9g etlcdb",
                args.format
            );
            std::process::exit(1);
        });
        let input_path = std::path::Path::new(&args.input);
        // dump doesn't support --exclude, pass empty slice
        let records = if input_path.is_dir() {
            read_etl_dir(&args.input, fmt, &[])
        } else {
            read_etl_file(&args.input, fmt)
        }
        .unwrap_or_else(|e| {
            eprintln!("failed to read ETL data from {}: {e}", args.input);
            std::process::exit(1);
        });
        // Wrap in a single anonymous batch so the rest of the code is uniform.
        vec![EtlBatch { tag: "?", records }]
    };

    let total_records: usize = batches.iter().map(|b| b.records.len()).sum();
    tracing::info!(total_records, batches = batches.len(), "records loaded");

    // ── Per-character inspection ─────────────────────────────────────────────
    for ch in &targets {
        inspect_char(*ch, &batches, &args.out, args.cols, args.verbose);
    }
}

// ─── Per-character work ───────────────────────────────────────────────────────

fn inspect_char(ch: char, batches: &[EtlBatch], out_dir: &str, cols: u32, verbose: bool) {
    let codepoint = ch as u32;
    let unicode_name = unicode_names2::name(ch)
        .map(|n| n.to_string())
        .unwrap_or_else(|| "<unnamed>".to_string());

    // Collect every matching record across all batches, tagging each with its
    // batch tag so the printout can show which dataset it came from.
    let matches: Vec<(&str, &EtlRecord)> = batches
        .iter()
        .flat_map(|b| {
            b.records
                .iter()
                .filter(|r| r.character == Some(ch))
                .map(move |r| (b.tag, r))
        })
        .collect();

    // ── Header ──────────────────────────────────────────────────────────────
    println!();
    println!("═══ U+{codepoint:04X}  '{ch}'  {unicode_name} ═══");

    if matches.is_empty() {
        println!("  (not found in any loaded dataset)");
        return;
    }

    // Per-tag breakdown.
    let mut tag_counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for (tag, _) in &matches {
        *tag_counts.entry(tag).or_insert(0) += 1;
    }
    for (tag, count) in &tag_counts {
        let sample = matches.iter().find(|(t, _)| t == tag).unwrap().1;
        println!(
            "  [{tag}]  {count} sample(s)  {}x{}px",
            sample.width, sample.height
        );
    }
    println!("  total: {} sample(s)", matches.len());

    // ── Verbose debugging output ─────────────────────────────────────────────
    if verbose {
        println!();
        println!("  [verbose] Detailed record info:");

        for (idx, (tag, rec)) in matches.iter().enumerate() {
            println!("\n  Sample #{idx}  [{tag}]");

            // Print source file info
            if let Some(ref source) = rec.source_file {
                println!("    source_file: {}", source);
            } else {
                println!("    source_file: <unknown>");
            }

            // Print raw code info
            println!("    raw_code: 0x{:04X}", rec.raw_code);

            // For K-type (60x60), the CO-59 code is in the range 4-59
            // col = raw_code >> 6, row = raw_code & 0x3F
            let col = (rec.raw_code >> 6) as u8;
            let row = (rec.raw_code & 0x3F) as u8;
            println!("    CO-59: col={}, row={}  (valid range: 4-59)", col, row);

            // Print character mapping
            if let Some(c) = rec.character {
                let cp = c as u32;
                println!("    character: U+{:04X} '{}'  (mapped)", cp, c);
            } else {
                println!("    character: <none>  (not in CO-59 table)");
            }

            // Print dimensions
            println!("    dimensions: {}x{}px", rec.width, rec.height);

            // Print pixel statistics
            if !rec.pixels.is_empty() {
                let min_pixel = rec.pixels.iter().min().unwrap();
                let max_pixel = rec.pixels.iter().max().unwrap();
                println!(
                    "    pixels: {} total, range [{min_pixel}..{max_pixel}]",
                    rec.pixels.len()
                );
            }

            // Print ASCII art for this sample
            println!("    ASCII art (40 cols):");
            print_ascii_art(&rec.pixels, rec.width, rec.height, 40);
        }
    }

    // ── ASCII-art preview of the first sample ───────────────────────────────
    let (first_tag, first_rec) = matches[0];
    println!();
    println!(
        "  First sample  [{first_tag}]  {}x{}  (ASCII art, ~40 cols)",
        first_rec.width, first_rec.height
    );
    print_ascii_art(&first_rec.pixels, first_rec.width, first_rec.height, 40);

    // ── Contact-sheet PNG ───────────────────────────────────────────────────
    let path = format!("{out_dir}/{codepoint:04x}.png");
    match write_contact_sheet(&matches, cols, &path) {
        Ok(()) => println!("\n  contact sheet -> {path}"),
        Err(e) => eprintln!("  warning: could not write contact sheet {path}: {e}"),
    }
}

// ─── ASCII art renderer ───────────────────────────────────────────────────────

/// Print a scaled-down ASCII-art representation to stdout.
///
/// The image is scaled to `target_cols` wide (preserving aspect ratio).
/// Pixels >= 128 are treated as background (space); pixels < 128 are foreground
/// (full-block `█`). The border is printed so misaligned images are obvious.
fn print_ascii_art(pixels: &[u8], w: u32, h: u32, target_cols: u32) {
    let scale = w as f32 / target_cols as f32;
    // Terminal cells are roughly 2x taller than wide, so squish height by 0.5.
    let target_rows = ((h as f32 / scale) * 0.5) as u32;
    let target_rows = target_rows.max(1);

    // Top border.
    let border_top = format!("  ┌{}┐", "─".repeat(target_cols as usize));
    println!("{border_top}");

    for ty in 0..target_rows {
        // Sample the source row at the centre of this scaled row.
        let sy = ((ty as f32 + 0.5) * scale * 2.0) as u32;
        let sy = sy.min(h - 1);

        print!("  │");
        for tx in 0..target_cols {
            let sx = ((tx as f32 + 0.5) * scale) as u32;
            let sx = sx.min(w - 1);
            let v = pixels[(sy * w + sx) as usize];
            // ETL images: dark strokes on light background.
            // v < 128 -> stroke -> show block; v >= 128 -> background -> space.
            print!("{}", if v < 128 { '█' } else { ' ' });
        }
        println!("│");
    }

    // Bottom border.
    let border_bot = format!("  └{}┘", "─".repeat(target_cols as usize));
    println!("{border_bot}");
}

// ─── Contact-sheet PNG ────────────────────────────────────────────────────────

/// Tile all matching samples into a single grayscale PNG.
///
/// Each cell is `w x h` pixels (native resolution of the records - all records
/// for a given character share the same dimensions since they come from the
/// same ETL family). Cells are arranged in `cols` columns with a 1-pixel
/// white separator border between cells.
fn write_contact_sheet(
    matches: &[(&str, &EtlRecord)],
    cols: u32,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if matches.is_empty() {
        return Ok(());
    }

    let cell_w = matches[0].1.width;
    let cell_h = matches[0].1.height;
    let n = matches.len() as u32;
    let cols = cols.min(n); // don't make cols wider than the sample count
    let rows = n.div_ceil(cols);

    // 1-pixel gap between cells.
    let gap = 1u32;
    let sheet_w = cols * cell_w + (cols + 1) * gap;
    let sheet_h = rows * cell_h + (rows + 1) * gap;

    let mut sheet: GrayImage = ImageBuffer::from_pixel(sheet_w, sheet_h, Luma([255u8]));

    for (idx, (_, rec)) in matches.iter().enumerate() {
        let idx = idx as u32;
        let col = idx % cols;
        let row = idx / cols;
        let ox = gap + col * (cell_w + gap);
        let oy = gap + row * (cell_h + gap);

        for y in 0..rec.height {
            for x in 0..rec.width {
                let v = rec.pixels[(y * rec.width + x) as usize];
                sheet.put_pixel(ox + x, oy + y, Luma([v]));
            }
        }
    }

    sheet.save(path)?;
    Ok(())
}
