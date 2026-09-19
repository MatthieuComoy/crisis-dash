//! Thumbnail fetch and downsample. No terminal graphics protocol (Kitty,
//! iTerm2, Sixel) is used or required: the image is shrunk to a handful of
//! pixels and handed to `ui::thumb` to draw as coloured half-block characters,
//! which renders identically in any terminal that already does truecolor —
//! which the rest of this UI assumes everywhere else.
//!
//! Deliberately decoupled from ratatui: this module only fetches bytes and
//! produces plain pixel data, so it has nothing terminal-shaped to get wrong.

use anyhow::{anyhow, Result};
use image::imageops::FilterType;

/// Thumbnail width in terminal columns.
pub const CELL_COLS: u32 = 44;
/// Thumbnail height in terminal rows. Each row covers two pixel-rows (a top
/// and bottom half-block), so the decoded image is `CELL_ROWS * 2` tall.
pub const CELL_ROWS: u32 = 14;

/// A small RGB raster, already sized for terminal display.
#[derive(Debug)]
pub struct Raster {
    pub cols: usize,
    /// Terminal rows this raster occupies once drawn as half-blocks.
    pub rows: usize,
    /// Row-major, `cols` wide and `2 * rows` tall.
    pixels: Vec<(u8, u8, u8)>,
}

impl Raster {
    pub fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8) {
        self.pixels[y * self.cols + x]
    }
}

/// What the UI knows about one thumbnail URL at any point in time.
#[derive(Debug)]
pub enum ThumbState {
    Loading,
    Ready(Raster),
    Failed,
}

/// Result of a background fetch, sent back to the event loop.
#[derive(Debug)]
pub enum ThumbMsg {
    Ready { url: String, raster: Raster },
    Failed { url: String },
}

/// Download and shrink one thumbnail. The original is discarded as soon as
/// the small raster is built, so nothing image-sized lives past this call.
pub async fn fetch_and_downsample(client: reqwest::Client, url: String) -> Result<Raster> {
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status().as_u16()));
    }
    let bytes = resp.bytes().await?;
    if bytes.is_empty() {
        return Err(anyhow!("empty body"));
    }

    let target_h = CELL_ROWS * 2;
    let img = image::load_from_memory(&bytes)?;
    // `resize_exact` ignores aspect ratio, which is fine here: every caller
    // draws into a fixed-size box, and a headline photo cropped slightly is
    // less jarring than a variable-height hole in the timeline.
    let resized = img.resize_exact(CELL_COLS, target_h, FilterType::Triangle).to_rgb8();

    let pixels = resized.pixels().map(|p| (p[0], p[1], p[2])).collect();
    Ok(Raster { cols: CELL_COLS as usize, rows: CELL_ROWS as usize, pixels })
}
