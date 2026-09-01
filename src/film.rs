//! Tone mapping and PNG output.

use glam::Vec3;

/// ACES filmic curve, Narkowicz's analytic fit. Cheap, and close enough to the
/// reference ACES RRT+ODT that reference comparisons stay meaningful.
#[inline]
fn aces(x: Vec3) -> Vec3 {
    const A: f32 = 2.51;
    const B: f32 = 0.03;
    const C: f32 = 2.43;
    const D: f32 = 0.59;
    const E: f32 = 0.14;
    let num = x * (x * A + Vec3::splat(B));
    let den = x * (x * C + Vec3::splat(D)) + Vec3::splat(E);
    (num / den).clamp(Vec3::ZERO, Vec3::ONE)
}

/// Linear -> sRGB transfer. The piecewise standard curve, not a bare 1/2.2
/// power, so comparisons against a reference renderer are not skewed in the
/// shadows.
#[inline]
fn srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let v = if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0 + 0.5) as u8
}

pub struct Film {
    pub width: u32,
    pub height: u32,
    pub data: Vec<Vec3>,
}

impl Film {
    pub fn new(width: u32, height: u32) -> Film {
        Film {
            width,
            height,
            data: vec![Vec3::ZERO; (width * height) as usize],
        }
    }

    /// `tonemap = false` writes the buffer straight through the sRGB transfer.
    /// Debug modes (normals, depth, step counts) are already display-referred,
    /// and pushing them through ACES would misrepresent them.
    pub fn encode(&self, exposure: f32, tonemap: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() * 3);
        for &c in &self.data {
            let scaled = c * exposure;
            let m = if tonemap { aces(scaled) } else { scaled };
            out.push(srgb(m.x));
            out.push(srgb(m.y));
            out.push(srgb(m.z));
        }
        out
    }

    pub fn save(&self, path: &str, exposure: f32, tonemap: bool) -> Result<(), String> {
        let buf = self.encode(exposure, tonemap);
        image::save_buffer(path, &buf, self.width, self.height, image::ColorType::Rgb8)
            .map_err(|e| format!("failed to write {}: {}", path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_transfer_hits_known_anchors() {
        assert_eq!(srgb(0.0), 0);
        assert_eq!(srgb(1.0), 255);
        // Mid-grey: linear 0.2140 encodes to sRGB ~0.5 (128/255).
        assert!(
            (srgb(0.2140) as i32 - 128).abs() <= 1,
            "got {}",
            srgb(0.2140)
        );
        // Out-of-range input must clamp, not wrap.
        assert_eq!(srgb(-5.0), 0);
        assert_eq!(srgb(9.0), 255);
    }

    #[test]
    fn srgb_is_monotonic() {
        let mut prev = 0u8;
        for i in 0..=1000 {
            let v = srgb(i as f32 / 1000.0);
            assert!(v >= prev, "sRGB transfer is not monotonic at {}", i);
            prev = v;
        }
    }

    /// ACES must compress highlights into range without ever going negative or
    /// exceeding 1, or the sRGB encode would wrap.
    #[test]
    fn aces_maps_into_unit_range() {
        for e in [0.0f32, 0.1, 0.5, 1.0, 4.0, 100.0, 1e6] {
            let c = aces(Vec3::splat(e));
            assert!(
                c.cmpge(Vec3::ZERO).all() && c.cmple(Vec3::ONE).all(),
                "aces({}) = {:?} escaped [0,1]",
                e,
                c
            );
        }
        assert!(
            aces(Vec3::splat(1.0)).x > aces(Vec3::splat(0.5)).x,
            "not monotonic"
        );
    }

    #[test]
    fn encode_produces_three_bytes_per_pixel() {
        let mut f = Film::new(4, 3);
        f.data[0] = Vec3::new(1.0, 0.0, 0.0);
        let buf = f.encode(1.0, true);
        assert_eq!(buf.len(), 4 * 3 * 3);
        assert!(buf[0] > buf[1], "red channel should dominate");
    }
}
