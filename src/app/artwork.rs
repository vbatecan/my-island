//! Deduplicate encoded artwork before UI dispatch and retain only a small raster.
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

const MAX_ENCODED_BYTES: usize = 1024 * 1024;
const MAX_EDGE: u32 = 256;

#[derive(Default)]
pub(super) struct ArtworkCache {
    encoded: Option<Vec<u8>>,
}

impl ArtworkCache {
    /// Compare actual bytes, so equal-size replacements and hash collisions cannot
    /// leave stale art. Oversized input is unsupported and acts like absent art.
    pub(super) fn changed(&mut self, bytes: Option<&[u8]>) -> bool {
        let bytes = bytes.filter(|bytes| bytes.len() <= MAX_ENCODED_BYTES);
        if self.encoded.as_deref() == bytes {
            return false;
        }
        self.encoded = bytes.map(<[u8]>::to_vec);
        true
    }
}

/// Decode once per encoded-art change. The original image and its full-size
/// raster are dropped before returning; the UI receives at most 256 KiB of RGBA.
pub(super) fn decode(bytes: &[u8]) -> Option<Image> {
    if bytes.is_empty() || bytes.len() > MAX_ENCODED_BYTES {
        return None;
    }
    let image = Image::load_from_data(bytes, None).ok()?;
    let pixels = image.to_rgba8()?;
    let (width, height) = fitted_size(pixels.width(), pixels.height())?;
    if (width, height) == (pixels.width(), pixels.height()) {
        return Some(Image::from_rgba8(pixels));
    }
    Some(Image::from_rgba8(resample(&pixels, width, height)))
}

fn fitted_size(width: u32, height: u32) -> Option<(u32, u32)> {
    let longest = width.max(height);
    if width == 0 || height == 0 {
        return None;
    }
    if longest <= MAX_EDGE {
        return Some((width, height));
    }
    let fit = |edge: u32| {
        ((edge as u64 * MAX_EDGE as u64 + longest as u64 / 2) / longest as u64).max(1) as u32
    };
    Some((fit(width), fit(height)))
}

/// Bilinear filtering in premultiplied-alpha space prevents invisible RGB values
/// from bleeding colored fringes into transparent edges. Slint receives straight
/// alpha after filtering, matching `Image::from_rgba8`'s contract.
fn resample(
    source: &SharedPixelBuffer<Rgba8Pixel>,
    width: u32,
    height: u32,
) -> SharedPixelBuffer<Rgba8Pixel> {
    let mut output = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    let input = source.as_bytes();
    let target = output.make_mut_bytes();
    let coordinate = |index: u32, from: u32, to: u32| {
        let position =
            ((index as f64 + 0.5) * from as f64 / to as f64 - 0.5).clamp(0., (from - 1) as f64);
        let lower = position.floor() as u32;
        (lower, (lower + 1).min(from - 1), position - lower as f64)
    };
    for y in 0..height {
        let (y0, y1, fy) = coordinate(y, source.height(), height);
        for x in 0..width {
            let (x0, x1, fx) = coordinate(x, source.width(), width);
            let samples = [
                (x0, y0, (1. - fx) * (1. - fy)),
                (x1, y0, fx * (1. - fy)),
                (x0, y1, (1. - fx) * fy),
                (x1, y1, fx * fy),
            ];
            let mut alpha = 0.;
            let mut color = [0.; 3];
            for (sx, sy, weight) in samples {
                let offset = (sy as usize * source.width() as usize + sx as usize) * 4;
                let weighted_alpha = input[offset + 3] as f64 * weight;
                alpha += weighted_alpha;
                for channel in 0..3 {
                    color[channel] += input[offset + channel] as f64 * weighted_alpha;
                }
            }
            let offset = (y as usize * width as usize + x as usize) * 4;
            target[offset + 3] = alpha.round().clamp(0., 255.) as u8;
            if alpha > 0. {
                for channel in 0..3 {
                    target[offset + channel] =
                        (color[channel] / alpha).round().clamp(0., 255.) as u8;
                }
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_preserves_aspect_ratio_without_upscaling_or_overflow() {
        for (input, expected) in [
            ((4096, 2048), (256, 128)),
            ((2048, 4096), (128, 256)),
            ((32, 56), (32, 56)),
            ((1, 10000), (1, 256)),
            ((u32::MAX, u32::MAX), (256, 256)),
        ] {
            assert_eq!(fitted_size(input.0, input.1), Some(expected));
            assert!(expected.0 * expected.1 * 4 <= 256 * 256 * 4);
        }
        assert_eq!(fitted_size(0, 12), None);
        assert_eq!(fitted_size(12, 0), None);
    }

    #[test]
    fn filter_preserves_color_and_correctly_weights_transparency() {
        let mut source = SharedPixelBuffer::<Rgba8Pixel>::new(2, 1);
        source
            .make_mut_bytes()
            .copy_from_slice(&[255, 0, 0, 0, 0, 0, 255, 255]);
        assert_eq!(resample(&source, 1, 1).as_bytes(), &[0, 0, 255, 128]);
        source
            .make_mut_bytes()
            .copy_from_slice(&[80, 120, 160, 255, 80, 120, 160, 255]);
        assert_eq!(resample(&source, 1, 1).as_bytes(), &[80, 120, 160, 255]);
        source.make_mut_bytes().fill(0);
        assert_eq!(resample(&source, 1, 1).as_bytes(), &[0, 0, 0, 0]);
    }

    #[test]
    fn cache_detects_equal_length_changes_and_releases_absent_art() {
        let mut cache = ArtworkCache::default();
        assert!(!cache.changed(None));
        assert!(cache.changed(Some(&[1, 2, 3])));
        assert!(!cache.changed(Some(&[1, 2, 3])));
        assert!(cache.changed(Some(&[1, 2, 4])));
        assert!(cache.changed(None));
        assert!(cache.encoded.is_none());
        assert!(!cache.changed(None));
        assert!(cache.changed(Some(&vec![1; MAX_ENCODED_BYTES])));
        assert!(cache.changed(Some(&vec![1; MAX_ENCODED_BYTES + 1])));
        assert!(cache.encoded.is_none());
        assert!(!cache.changed(Some(&vec![2; MAX_ENCODED_BYTES + 1])));
    }

    #[test]
    fn invalid_encoded_input_is_rejected() {
        assert!(decode(&[]).is_none());
        assert!(decode(b"not an image").is_none());
        assert!(decode(&vec![0; MAX_ENCODED_BYTES + 1]).is_none());
    }
}
