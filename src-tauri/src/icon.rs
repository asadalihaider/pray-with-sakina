//! The brand mark: an 8-point star drawn as two overlapping squares, one
//! rotated 45 degrees, in a single stroke weight. Rasterised at runtime as
//! black + alpha so macOS can treat it as a template image and invert it
//! for light and dark menu bars.

const HALF_SIDE: f32 = 0.62;
const STROKE_HALF: f32 = 0.085;

/// Unsigned distance from a point to a centred square's outline.
fn distance_to_square_outline(x: f32, y: f32, half_side: f32) -> f32 {
    let dx = x.abs() - half_side;
    let dy = y.abs() - half_side;
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    (outside + dx.max(dy).min(0.0)).abs()
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn star_rgba(size: u32) -> Vec<u8> {
    let mut buffer = vec![0u8; (size * size * 4) as usize];
    let centre = size as f32 / 2.0;
    // One normalised unit spans centre -> edge, less a small margin.
    let unit = centre / 1.10;
    let antialias = 1.0 / unit;
    let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;

    for y in 0..size {
        for x in 0..size {
            let px = (x as f32 + 0.5 - centre) / unit;
            let py = (y as f32 + 0.5 - centre) / unit;

            let axis_aligned = distance_to_square_outline(px, py, HALF_SIDE);
            let rotated = distance_to_square_outline(
                (px + py) * inv_sqrt2,
                (px - py) * inv_sqrt2,
                HALF_SIDE,
            );
            let distance = axis_aligned.min(rotated);

            let coverage = smoothstep(
                STROKE_HALF + antialias,
                STROKE_HALF - antialias,
                distance,
            );

            let index = ((y * size + x) * 4) as usize;
            buffer[index + 3] = (coverage * 255.0).round() as u8;
        }
    }

    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(buffer: &[u8], size: u32, x: u32, y: u32) -> u8 {
        buffer[((y * size + x) * 4 + 3) as usize]
    }

    #[test]
    fn stroke_is_drawn_and_centre_is_hollow() {
        let size = 44;
        let star = star_rgba(size);

        // The middle of the star is empty — it is an outline, not a fill.
        assert_eq!(alpha_at(&star, size, size / 2, size / 2), 0);
        // The top edge of the axis-aligned square is inked.
        let edge_y = (size as f32 / 2.0 - HALF_SIDE * (size as f32 / 2.0 / 1.10)) as u32;
        assert!(alpha_at(&star, size, size / 2, edge_y) > 200);
    }

    #[test]
    fn mark_stays_inside_its_bounds() {
        let size = 44;
        let star = star_rgba(size);
        for i in 0..size {
            assert_eq!(alpha_at(&star, size, i, 0), 0);
            assert_eq!(alpha_at(&star, size, i, size - 1), 0);
            assert_eq!(alpha_at(&star, size, 0, i), 0);
            assert_eq!(alpha_at(&star, size, size - 1, i), 0);
        }
    }

    #[test]
    fn shape_has_eight_fold_symmetry() {
        let size = 44;
        let star = star_rgba(size);
        // Mirroring and transposing must both land on the same shape, which
        // is what makes the two squares read as one 8-point star.
        for y in 0..size {
            for x in 0..size {
                let a = alpha_at(&star, size, x, y);
                assert_eq!(a, alpha_at(&star, size, size - 1 - x, y));
                assert_eq!(a, alpha_at(&star, size, x, size - 1 - y));
                assert_eq!(a, alpha_at(&star, size, y, x));
            }
        }
    }
}
