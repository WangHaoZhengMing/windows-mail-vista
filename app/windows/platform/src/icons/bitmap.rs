//! Decode named PNG assets once; resize in premultiplied color space for DPI.
use super::IconSet;
use std::{io::Cursor, sync::OnceLock};
const TILE_SIDE: usize = 128;
static TOOLBAR: OnceLock<Result<Vec<Vec<u8>>, String>> = OnceLock::new();
static MESSAGES: OnceLock<Result<Vec<Vec<u8>>, String>> = OnceLock::new();
static FOLDERS: OnceLock<Result<Vec<Vec<u8>>, String>> = OnceLock::new();
fn decode(name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("{name}: {e}"))?;
    if (reader.info().width, reader.info().height) != (128, 128) {
        return Err(format!("{name}: expected a 128x128 icon"));
    }
    let mut rgba = vec![0; reader.output_buffer_size().ok_or("Icon too large")?];
    let frame = reader
        .next_frame(&mut rgba)
        .map_err(|e| format!("{name}: {e}"))?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return Err(format!("{name}: expected 8-bit RGBA"));
    }
    Ok(rgba[..frame.buffer_size()]
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(premultiplied_bgra)
        .collect())
}
pub(super) fn strip(set: IconSet, size: usize) -> Result<Vec<u8>, String> {
    let cache = match set {
        IconSet::Toolbar => &TOOLBAR,
        IconSet::Folders => &MESSAGES,
        IconSet::FolderTree => &FOLDERS,
    };
    let tiles = cache
        .get_or_init(|| {
            set.assets()
                .iter()
                .map(|(name, bytes)| decode(name, bytes))
                .collect()
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let mut strip = vec![0; size * size * set.count() * 4];
    let resize = Resize::new(TILE_SIDE, size);
    for (index, source) in tiles.iter().enumerate() {
        let tile = resize.apply(source);
        for y in 0..size {
            let start = (y * size * set.count() + index * size) * 4;
            strip[start..start + size * 4].copy_from_slice(&tile[y * size * 4..(y + 1) * size * 4]);
        }
    }
    Ok(strip)
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
    #[test]
    fn independent_assets_match_original_artwork_pixel_for_pixel() {
        {
            let mut reader = png::Decoder::new(Cursor::new(include_bytes!(
                "../../../../../assets/windows/sources/legacy-runtime-atlases/toolbar.png"
            )))
            .read_info()
            .unwrap();
            let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut rgba).unwrap();
            let width = frame.width as usize;
            let set = IconSet::Toolbar;
            for (asset, original) in [0, 3, 4, 5, 9, 10, 22, 29, 32, 33, 35, 43, 44, 45, 49]
                .into_iter()
                .enumerate()
            {
                let (name, bytes) = set.assets()[asset];
                let tile = decode(name, bytes).unwrap();
                for y in 0..128 {
                    for x in 0..128 {
                        let start = ((original / 8 * 128 + y) * width + original % 8 * 128 + x) * 4;
                        assert_eq!(
                            &tile[(y * 128 + x) * 4..(y * 128 + x + 1) * 4],
                            &premultiplied_bgra(rgba[start..start + 4].try_into().unwrap()),
                            "{name} at {x},{y}"
                        );
                    }
                }
            }
        }
        {
            let mut reader = png::Decoder::new(Cursor::new(include_bytes!(
                "../../../../../assets/windows/sources/legacy-runtime-atlases/folders.png"
            )))
            .read_info()
            .unwrap();
            let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut rgba).unwrap();
            let width = frame.width as usize;
            let set = IconSet::Folders;
            for (asset, original) in [9, 10].into_iter().enumerate() {
                let (name, bytes) = set.assets()[asset];
                let tile = decode(name, bytes).unwrap();
                for y in 0..128 {
                    for x in 0..128 {
                        let start =
                            ((original / 10 * 128 + y) * width + original % 10 * 128 + x) * 4;
                        assert_eq!(
                            &tile[(y * 128 + x) * 4..(y * 128 + x + 1) * 4],
                            &premultiplied_bgra(rgba[start..start + 4].try_into().unwrap()),
                            "{name} at {x},{y}"
                        );
                    }
                }
            }
        }
        {
            let mut reader = png::Decoder::new(Cursor::new(include_bytes!(
                "../../../../../assets/windows/sources/legacy-runtime-atlases/folder-tree.png"
            )))
            .read_info()
            .unwrap();
            let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut rgba).unwrap();
            let width = frame.width as usize;
            let set = IconSet::FolderTree;
            for (asset, original) in [0, 1, 2, 3, 4, 5, 6, 7, 8].into_iter().enumerate() {
                let (name, bytes) = set.assets()[asset];
                let tile = decode(name, bytes).unwrap();
                for y in 0..128 {
                    for x in 0..128 {
                        let start = ((original / 3 * 128 + y) * width + original % 3 * 128 + x) * 4;
                        assert_eq!(
                            &tile[(y * 128 + x) * 4..(y * 128 + x + 1) * 4],
                            &premultiplied_bgra(rgba[start..start + 4].try_into().unwrap()),
                            "{name} at {x},{y}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn named_assets_scale_with_valid_premultiplied_alpha() {
        for set in [IconSet::Toolbar, IconSet::Folders, IconSet::FolderTree] {
            for size in [16, 20, 24, 32, 48, 128, 256] {
                let pixels = strip(set, size).unwrap();
                assert_eq!(pixels.len(), size * size * set.count() * 4);
                assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 0));
                for p in pixels.as_chunks::<4>().0 {
                    assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
                }
            }
        }
    }
    #[test]
    fn invalid_assets_are_rejected() {
        assert!(decode("broken", b"not PNG").is_err());
    }
}
