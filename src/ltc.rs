//! Analytic polygonal area-light integration.
//!
//! The diffuse term here is *exact*, not approximate. The cosine-weighted solid
//! angle subtended by a polygon has a closed form (Lambert, 1760), and it is
//! the degenerate case of Linearly Transformed Cosines with the identity
//! transform. Evaluating it costs one `acos` per edge and returns the fully
//! converged answer — where a path tracer needs hundreds of shadow-ray samples
//! per pixel and still leaves visible noise.
//!
//! The specular term uses the representative-point approximation (Karis 2013)
//! rather than full LTC. Full LTC needs a fitted 64x64 table of 3x3 matrices;
//! the representative point is analytic, noise-free, and accurate to a few
//! percent for the roughness range this renderer targets. Swapping in the
//! fitted tables is a drop-in upgrade to this one function.

use glam::Vec3;

const INV_2PI: f32 = 0.159_154_94;

/// Sutherland-Hodgman clip of a quad against the horizon plane `dot(v, n) = 0`.
///
/// Without this, light polygons that straddle the shading plane contribute
/// negative solid angle and the surface goes black in a ring around the light.
/// Returns the vertex count; the array is filled from index 0.
fn clip_to_horizon(l: &[Vec3; 4], out: &mut [Vec3; 5]) -> usize {
    let mut n = 0usize;
    for i in 0..4 {
        let a = l[i];
        let b = l[(i + 1) % 4];
        let da = a.z;
        let db = b.z;
        if da >= 0.0 {
            out[n] = a;
            n += 1;
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            out[n] = a + (b - a) * t;
            n += 1;
        }
        if n >= 5 {
            break;
        }
    }
    n
}

/// Cosine-weighted solid angle of a quad as seen from the origin of a local
/// frame whose +Z is the surface normal. Result is in [0, 1]: the fraction of
/// the cosine-weighted hemisphere the light covers.
///
/// `l` must already be in that local frame (light vertices minus shading point,
/// rotated into tangent space).
pub fn quad_form_factor(l: &[Vec3; 4]) -> f32 {
    let mut clipped = [Vec3::ZERO; 5];
    let n = clip_to_horizon(l, &mut clipped);
    if n < 3 {
        return 0.0;
    }

    // Normalise onto the unit sphere, then sum the signed edge contributions.
    for v in clipped.iter_mut().take(n) {
        let len = v.length();
        if len > 1e-9 {
            *v /= len;
        }
    }

    let mut sum = 0.0f32;
    for i in 0..n {
        let a = clipped[i];
        let b = clipped[(i + 1) % n];
        let cos_t = a.dot(b).clamp(-1.0, 1.0);
        let theta = cos_t.acos();
        let cross = a.cross(b);
        let len = cross.length();
        if len > 1e-9 {
            // Only the z component survives the dot with the local normal.
            sum += theta * (cross.z / len);
        }
    }
    (sum * INV_2PI).max(0.0)
}

/// Orthonormal tangent frame with +Z along `n`. Duff et al.'s branchless
/// construction: no trig, no degenerate case at the poles.
#[inline]
pub fn frame(n: Vec3) -> (Vec3, Vec3) {
    let s = if n.z >= 0.0 { 1.0f32 } else { -1.0f32 };
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    (
        Vec3::new(1.0 + s * n.x * n.x * a, s * b, -s * n.x),
        Vec3::new(b, s + n.y * n.y * a, -n.y),
    )
}

/// Transform light vertices into the shading point's tangent frame.
///
/// The winding is reversed on the way in. The DSL specifies light vertices
/// counter-clockwise *as seen from the emitting side*, which is the intuitive
/// way to author a downward-facing ceiling panel. But `quad_form_factor` sums
/// signed edge contributions, so it needs the polygon counter-clockwise *as
/// seen from the receiver* — the opposite sense. Without this flip every form
/// factor comes out negative and is clamped to zero, and the scene renders
/// black with no other symptom.
#[inline]
pub fn to_local(verts: &[Vec3; 4], p: Vec3, t: Vec3, b: Vec3, n: Vec3) -> [Vec3; 4] {
    let mut out = [Vec3::ZERO; 4];
    for i in 0..4 {
        let d = verts[3 - i] - p;
        out[i] = Vec3::new(d.dot(t), d.dot(b), d.dot(n));
    }
    out
}

/// Closest point to `q` inside the quad, in the quad's own plane. Used to pick
/// the representative point for the specular lobe.
pub fn closest_on_quad(verts: &[Vec3; 4], q: Vec3) -> Vec3 {
    let o = verts[0];
    let e1 = verts[1] - o;
    let e2 = verts[3] - o;
    let d = q - o;
    let u = (d.dot(e1) / e1.dot(e1)).clamp(0.0, 1.0);
    let v = (d.dot(e2) / e2.dot(e2)).clamp(0.0, 1.0);
    o + e1 * u + e2 * v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ceiling panel authored the way the DSL requires: counter-clockwise as
    /// seen from the emitting side, which for a downward-facing light means
    /// seen from below.
    fn ceiling_light(half: f32, h: f32) -> [Vec3; 4] {
        [
            Vec3::new(-half, h, -half),
            Vec3::new(half, h, -half),
            Vec3::new(half, h, half),
            Vec3::new(-half, h, half),
        ]
    }

    fn form_factor_at(verts: &[Vec3; 4], p: Vec3, n: Vec3) -> f32 {
        let (t, b) = frame(n);
        quad_form_factor(&to_local(verts, p, t, b, n))
    }

    /// Guards the winding bug. `quad_form_factor` sums *signed* edge
    /// contributions, so it needs the polygon counter-clockwise as seen from the
    /// receiver — the opposite sense to how the DSL authors it. Without the flip
    /// in `to_local` every form factor comes out negative, gets clamped to zero,
    /// and the scene renders black with no other symptom.
    #[test]
    fn dsl_winding_yields_positive_form_factor() {
        let ff = form_factor_at(&ceiling_light(1.0, 2.0), Vec3::ZERO, Vec3::Y);
        assert!(
            ff > 0.05,
            "form factor collapsed to {} — light winding is reversed",
            ff
        );
    }

    /// Quantitative check against the analytic limit: a small patch of area A at
    /// distance d directly overhead has cosine-weighted form factor A / (pi d^2).
    /// Catches a sign flip that a positivity test alone would miss.
    #[test]
    fn small_distant_light_matches_analytic_limit() {
        let (half, h) = (0.05f32, 5.0f32);
        let ff = form_factor_at(&ceiling_light(half, h), Vec3::ZERO, Vec3::Y);
        let expect = (2.0 * half) * (2.0 * half) / (std::f32::consts::PI * h * h);
        let rel = (ff - expect).abs() / expect;
        assert!(
            rel < 0.02,
            "ff {} vs analytic {} (rel err {:.4})",
            ff,
            expect,
            rel
        );
    }

    /// A light entirely below the horizon must contribute nothing, rather than
    /// a negative value that would darken the surface.
    #[test]
    fn light_below_horizon_contributes_nothing() {
        let ff = form_factor_at(&ceiling_light(1.0, -2.0), Vec3::ZERO, Vec3::Y);
        assert!(ff < 1e-6, "light below the horizon returned {}", ff);
    }
}
