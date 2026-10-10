//! Embedded artwork atlases, decoded once and resized per independent tile.

use super::IconSet;
use std::{io::Cursor, sync::OnceLock};

const TILE_SIDE: usize = 128;
const TOOLBAR_PNG: &[u8] = include_bytes!("../../../../../assets/windows/icons/toolbar.png");
const FOLDERS_PNG: &[u8] = include_bytes!("../../../../../assets/windows/icons/folders.png");
const TREE_PNG: &[u8] = include_bytes!("../../../../../assets/windows/icons/folder-tree.png");

static TOOLBAR_ATLAS: OnceLock<Result<Atlas, String>> = OnceLock::new();
static FOLDERS_ATLAS: OnceLock<Result<Atlas, String>> = OnceLock::new();
static TREE_ATLAS: OnceLock<Result<Atlas, String>> = OnceLock::new();

struct Atlas {
    set: IconSet,
    /// Independent tiles in the original row-major index order, premultiplied BGRA.
    tiles: Vec<Vec<u8>>,
}

fn source(set: IconSet) -> &'static [u8] {
    match set {
        IconSet::Toolbar => TOOLBAR_PNG,
        IconSet::Folders => FOLDERS_PNG,
        IconSet::FolderTree => TREE_PNG,
    }
}

pub(super) fn strip(set: IconSet, size: usize) -> Result<Vec<u8>, String> {
    let cache = match set {
        IconSet::Toolbar => &TOOLBAR_ATLAS,
        IconSet::Folders => &FOLDERS_ATLAS,
        IconSet::FolderTree => &TREE_ATLAS,
    };
    let atlas = cache
        .get_or_init(|| Atlas::decode(set, source(set)))
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(atlas.strip(size))
}

impl Atlas {
    fn decode(set: IconSet, bytes: &[u8]) -> Result<Self, String> {
        let (columns, rows) = set.grid();
        let width = columns * TILE_SIDE;
        let height = rows * TILE_SIDE;
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
        let info = reader.info();
        if (info.width, info.height) != (width as u32, height as u32) {
            return Err(format!("{set:?} atlas must be {width} by {height} pixels"));
        }
        let length = reader
            .output_buffer_size()
            .ok_or("Icon atlas output buffer is too large")?;
        let mut rgba = vec![0; length];
        let frame = reader
            .next_frame(&mut rgba)
            .map_err(|error| error.to_string())?;
        if frame.color_type != png::ColorType::Rgba
            || frame.bit_depth != png::BitDepth::Eight
            || frame.buffer_size() != width * height * 4
        {
            return Err(format!(
                "{set:?} atlas must contain a complete 8-bit RGBA image"
            ));
        }
        Ok(Self::from_rgba(set, &rgba[..frame.buffer_size()]))
    }

    fn from_rgba(set: IconSet, rgba: &[u8]) -> Self {
        let (columns, _) = set.grid();
        let width = columns * TILE_SIDE;
        let tiles = (0..set.count())
            .map(|index| {
                let left = (index % columns) * TILE_SIDE;
                let top = (index / columns) * TILE_SIDE;
                let mut tile = Vec::with_capacity(TILE_SIDE * TILE_SIDE * 4);
                for y in top..top + TILE_SIDE {
                    let start = (y * width + left) * 4;
                    for pixel in rgba[start..start + TILE_SIDE * 4].as_chunks::<4>().0 {
                        tile.extend_from_slice(&premultiplied_bgra(pixel));
                    }
                }
                tile
            })
            .collect();
        Self { set, tiles }
    }

    fn strip(&self, size: usize) -> Vec<u8> {
        let count = self.set.count();
        let mut strip = vec![0; size * size * count * 4];
        let resize = Resize::new(TILE_SIDE, size);
        for (index, source) in self.tiles.iter().enumerate() {
            let tile = resize.apply(source);
            for y in 0..size {
                let start = (y * size * count + index * size) * 4;
                strip[start..start + size * 4]
                    .copy_from_slice(&tile[y * size * 4..(y + 1) * size * 4]);
            }
        }
        strip
    }
}

fn premultiplied_bgra(rgba: &[u8; 4]) -> [u8; 4] {
    let alpha = u16::from(rgba[3]);
    let multiply = |color: u8| ((u16::from(color) * alpha + 127) / 255) as u8;
    [
        multiply(rgba[2]),
        multiply(rgba[1]),
        multiply(rgba[0]),
        rgba[3],
    ]
}

/// Separable triangle filter in premultiplied color space. Downsampling widens
/// the support to cover the source footprint, rather than taking a single sample.
/// All taps stay within a tile, so adjacent atlas artwork cannot leak into it.
struct Resize {
    source: usize,
    target: usize,
    taps: Vec<Vec<(usize, f32)>>,
}

impl Resize {
    fn new(source: usize, target: usize) -> Self {
        let ratio = source as f32 / target as f32;
        let support = ratio.max(1.0);
        let taps = (0..target)
            .map(|position| {
                let center = (position as f32 + 0.5) * ratio - 0.5;
                let first = (center - support).ceil().max(0.0) as usize;
                let last = ((center + support).floor() as usize).min(source - 1);
                let mut taps: Vec<_> = (first..=last)
                    .map(|sample| {
                        (
                            sample,
                            (1.0 - (sample as f32 - center).abs() / support).max(0.0),
                        )
                    })
                    .collect();
                let total: f32 = taps.iter().map(|(_, weight)| weight).sum();
                for (_, weight) in &mut taps {
                    *weight /= total;
                }
                taps
            })
            .collect();
        Self {
            source,
            target,
            taps,
        }
    }

    fn apply(&self, pixels: &[u8]) -> Vec<u8> {
        if self.source == self.target {
            return pixels.to_vec();
        }
        let mut horizontal = vec![[0.0; 4]; self.source * self.target];
        for y in 0..self.source {
            for (x, taps) in self.taps.iter().enumerate() {
                let output = &mut horizontal[y * self.target + x];
                for &(sample, weight) in taps {
                    let start = (y * self.source + sample) * 4;
                    for channel in 0..4 {
                        output[channel] += pixels[start + channel] as f32 * weight;
                    }
                }
            }
        }
        let mut output = vec![0; self.target * self.target * 4];
        for (y, taps) in self.taps.iter().enumerate() {
            for x in 0..self.target {
                let mut pixel = [0.0; 4];
                for &(sample, weight) in taps {
                    for (channel, value) in pixel.iter_mut().enumerate() {
                        *value += horizontal[sample * self.target + x][channel] * weight;
                    }
                }
                let alpha = pixel[3].round().clamp(0.0, 255.0) as u8;
                let start = (y * self.target + x) * 4;
                for channel in 0..3 {
                    output[start + channel] = pixel[channel].round().clamp(0.0, alpha as f32) as u8;
                }
                output[start + 3] = alpha;
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETS: [IconSet; 3] = [IconSet::Toolbar, IconSet::Folders, IconSet::FolderTree];

    #[test]
    fn embedded_pngs_keep_the_original_indices_and_artwork() {
        assert_eq!(IconSet::Toolbar.count(), 64);
        assert_eq!(IconSet::Folders.count(), 60);
        for set in SETS {
            let atlas = Atlas::decode(set, source(set)).unwrap();
            assert_eq!(atlas.tiles.len(), set.count());
            let (columns, _) = set.grid();
            let width = columns * TILE_SIDE;
            let mut decoder = png::Decoder::new(Cursor::new(source(set)));
            decoder
                .set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
            let mut reader = decoder.read_info().unwrap();
            let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut rgba).unwrap();
            let source_pixels = rgba.as_chunks::<4>().0;
            for (index, tile) in atlas.tiles.iter().enumerate() {
                let pixels = tile.as_chunks::<4>().0;
                assert!(
                    pixels.iter().any(|pixel| pixel[3] != 0),
                    "set={set:?}, index={index}"
                );
                assert_eq!(Resize::new(TILE_SIDE, TILE_SIDE).apply(tile), *tile);
                for y in 0..TILE_SIDE {
                    for x in 0..TILE_SIDE {
                        let source_x = index % columns * TILE_SIDE + x;
                        let source_y = index / columns * TILE_SIDE + y;
                        let expected =
                            premultiplied_bgra(&source_pixels[source_y * width + source_x]);
                        assert_eq!(
                            pixels[y * TILE_SIDE + x],
                            expected,
                            "set={set:?}, index={index}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn resizing_cannot_mix_neighboring_atlas_tiles() {
        for set in SETS {
            let (columns, rows) = set.grid();
            let width = columns * TILE_SIDE;
            let height = rows * TILE_SIDE;
            let mut rgba = vec![0; width * height * 4];
            for y in 0..height {
                for x in 0..width {
                    let index = y / TILE_SIDE * columns + x / TILE_SIDE;
                    let start = (y * width + x) * 4;
                    rgba[start..start + 4].copy_from_slice(&[
                        index as u8,
                        0,
                        255 - index as u8,
                        255,
                    ]);
                }
            }
            let atlas = Atlas::from_rgba(set, &rgba);
            for size in [16, 20, 48, 128, 256] {
                let strip = atlas.strip(size);
                let pixels = strip.as_chunks::<4>().0;
                for y in 0..size {
                    for index in 0..set.count() {
                        for x in 0..size {
                            let offset = y * size * set.count() + index * size + x;
                            assert_eq!(
                                pixels[offset],
                                [255 - index as u8, 0, index as u8, 255],
                                "set={set:?}, index={index}, size={size}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn transparent_rgb_is_removed_before_scaling_without_color_fringes() {
        let source: Vec<_> = [
            [255, 255, 255, 0],
            [0, 0, 255, 0],
            [255, 0, 0, 255],
            [0, 255, 0, 0],
        ]
        .iter()
        .flat_map(premultiplied_bgra)
        .collect();
        for size in [1, 3, 16] {
            let resized = Resize::new(2, size).apply(&source);
            let pixels = resized.as_chunks::<4>().0;
            assert!(pixels.iter().any(|pixel| pixel[3] > 0 && pixel[3] < 255));
            for pixel in pixels {
                assert_eq!(pixel[0], 0);
                assert_eq!(pixel[1], 0);
                assert_eq!(pixel[2], pixel[3]);
            }
        }
    }

    #[test]
    fn both_sets_retain_transparency_and_premultiplied_alpha_at_all_scales() {
        for set in SETS {
            let atlas = Atlas::decode(set, source(set)).unwrap();
            for size in [16, 20, 24, 28, 32, 48, 64, 128, 256] {
                let resize = Resize::new(TILE_SIDE, size);
                for (index, tile) in atlas.tiles.iter().enumerate() {
                    let resized = resize.apply(tile);
                    let pixels = resized.as_chunks::<4>().0;
                    assert!(
                        pixels.iter().any(|pixel| pixel[3] == 0),
                        "set={set:?}, index={index}, size={size}: transparent background"
                    );
                    assert!(
                        pixels.iter().any(|pixel| pixel[3] != 0),
                        "set={set:?}, index={index}, size={size}: nonempty artwork"
                    );
                    // Native-size pixels are preserved and may have binary alpha.
                    if size != TILE_SIDE {
                        assert!(
                            pixels.iter().any(|pixel| pixel[3] > 0 && pixel[3] < 255),
                            "set={set:?}, index={index}, size={size}: filtered edge coverage"
                        );
                    }
                    assert!(
                        pixels
                            .iter()
                            .all(|pixel| pixel[..3].iter().all(|&color| color <= pixel[3])),
                        "set={set:?}, index={index}, size={size}: premultiplied channels"
                    );
                }
            }
        }
    }

    #[test]
    fn malformed_png_returns_an_error_without_panicking() {
        for set in SETS {
            assert!(Atlas::decode(set, b"not a PNG").is_err());
        }
    }

    #[test]
    fn an_atlas_with_the_other_sets_dimensions_is_rejected() {
        assert!(Atlas::decode(IconSet::Folders, TOOLBAR_PNG).is_err());
        assert!(Atlas::decode(IconSet::Toolbar, FOLDERS_PNG).is_err());
    }
}
