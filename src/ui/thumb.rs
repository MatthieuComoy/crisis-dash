//! Renders a `Raster` as rows of Unicode half-block spans. Each terminal cell
//! shows two source pixels: the upper-half-block glyph foreground is the top
//! pixel, its background the bottom one.

use crate::thumb::Raster;
use ratatui::prelude::*;

/// One row per output line, capped to `max_cols` so a photo never pushes the
/// timeline's own word-wrap width.
pub fn rows(raster: &Raster, max_cols: usize) -> Vec<Vec<Span<'static>>> {
    let cols = raster.cols.min(max_cols.max(1));
    (0..raster.rows)
        .map(|row| {
            (0..cols)
                .map(|col| {
                    let (tr, tg, tb) = raster.pixel(col, row * 2);
                    let (br, bg, bb) = raster.pixel(col, row * 2 + 1);
                    Span::styled(
                        "▀",
                        Style::default().fg(Color::Rgb(tr, tg, tb)).bg(Color::Rgb(br, bg, bb)),
                    )
                })
                .collect()
        })
        .collect()
}
