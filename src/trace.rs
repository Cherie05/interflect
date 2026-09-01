//! Sphere tracing, cone-traced soft shadows, and ambient occlusion.
//!
//! All three are noise-free: they read the distance field directly rather than
//! sampling it stochastically. That is the whole reason this renderer needs no
//! denoiser — there is nothing random to denoise.

use crate::scene::Scene;
use glam::Vec3;

pub const MAX_STEPS: u32 = 128;
pub const SHADOW_STEPS: u32 = 96;

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub t: f32,
    pub p: Vec3,
    pub n: Vec3,
    pub mat: u32,
}

/// Sphere trace a ray against the scene SDF.
///
/// Returns the hit and the number of distance evaluations consumed, which the
/// renderer accumulates so the published cost-per-pixel figure is measured
/// rather than estimated.
#[inline]
pub fn trace(scene: &Scene, ro: Vec3, rd: Vec3, tmin: f32, tmax: f32) -> (Option<Hit>, u32) {
    let mut t = tmin;
    let mut steps = 0u32;

    while steps < MAX_STEPS {
        let p = ro + rd * t;
        let h = scene.dist_mat(p);
        steps += 1;

        // Relative epsilon: distant surfaces do not need absolute precision,
        // and a fixed epsilon there wastes steps chasing sub-pixel detail.
        let eps = 1e-4 * t.max(1.0);
        if h.d < eps {
            let n = scene.normal(p);
            return (
                Some(Hit {
                    t,
                    p,
                    n,
                    mat: h.mat,
                }),
                steps + 4, // the normal costs four extra taps
            );
        }

        t += h.d;
        if t > tmax {
            break;
        }
    }
    (None, steps)
}

/// Cone-traced soft shadow (Quilez).
///
/// The distance field value at each step *is* a lower bound on the angular gap
/// to the nearest occluder, so `k * h / t` estimates penumbra coverage directly.
/// A stochastic shadow ray would need hundreds of samples for the same result.
/// `k` is the light's angular tightness: larger = sharper.
/// Two corrections over the naive `min(k * h / t)` form, both of which were
/// visible defects before they were applied:
///
/// 1. **Closest-approach correction.** The naive form samples the distance
///    field only at discrete `t`, so the penumbra estimate steps in discrete
///    jumps and the image shows concentric contour bands across every lit
///    surface. Interpolating the ray's closest approach between consecutive
///    samples (using the previous distance `ph`) recovers a continuous estimate.
///
/// 2. **The march must stop short of the light.** `h` is an omnidirectional
///    distance: it measures the nearest surface in *any* direction, not just
///    ahead. A ceiling-mounted light sits in the ceiling plane, so a shadow ray
///    approaching it runs nearly parallel to that plane, reports a tiny `h`, and
///    darkens a surface that is in full view of the light. Ending the march a
///    light-radius short of the source removes the false occlusion without
///    missing any real occluder, since nothing can be between the light and
///    itself.
#[inline]
pub fn soft_shadow(scene: &Scene, ro: Vec3, rd: Vec3, tmin: f32, tmax: f32, k: f32) -> (f32, u32) {
    let mut res = 1.0f32;
    let mut t = tmin;
    let mut steps = 0u32;
    let mut ph = 1e20f32;

    while steps < SHADOW_STEPS && t < tmax {
        let h = scene.dist(ro + rd * t);
        steps += 1;

        if h < 1e-4 {
            return (0.0, steps);
        }

        // Closest approach of the segment [t - step, t] to the surface.
        //
        // The correction is only valid while the ray is *approaching* a surface.
        // When it is receding (`h >= ph`) the nearest approach along the last
        // segment was at the previous sample, and applying the formula anyway is
        // degenerate: a ray moving directly away from a plane doubles `h` each
        // step, which makes `y == h`, `d == 0`, and drives visibility to zero on
        // a surface in plain view of the light.
        let (y, d) = if h < ph {
            let y = h * h / (2.0 * ph);
            (y, (h * h - y * y).max(0.0).sqrt())
        } else {
            (0.0, h)
        };
        res = res.min(k * d / (t - y).max(1e-4));
        if res < 1e-3 {
            return (0.0, steps);
        }

        ph = h;
        // Clamp the step so thin occluders are not stepped over.
        // A coarse maximum step leaves visible contour rings in the penumbra,
        // since `res` only updates at sample points.
        t += h.clamp(0.006, 0.12);
    }
    (res.clamp(0.0, 1.0), steps)
}

/// Ray/quad intersection, used to draw emitters as visible geometry.
#[inline]
pub fn intersect_quad(verts: &[Vec3; 4], ro: Vec3, rd: Vec3) -> Option<f32> {
    let e1 = verts[1] - verts[0];
    let e2 = verts[3] - verts[0];
    let n = e1.cross(e2);
    let denom = n.dot(rd);
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = (verts[0] - ro).dot(n) / denom;
    if t < 1e-4 {
        return None;
    }
    let p = ro + rd * t - verts[0];
    let u = p.dot(e1) / e1.dot(e1);
    let v = p.dot(e2) / e2.dot(e2);
    if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
        Some(t)
    } else {
        None
    }
}

/// Cosine-weighted fraction of the hemisphere above `(p, n)` that sees no
/// geometry at all.
///
/// This exists to calibrate the form-factor matrix. The point-to-disc form
/// factor underestimates clusters that subtend a large solid angle — measured
/// row sums came out at 0.21 in a closed box where the true value is exactly
/// 1.0, leaving the scene four times too dark. Rather than chase per-cluster
/// accuracy (that is hierarchical radiosity, and a much larger piece of work),
/// each row is rescaled so its total matches the geometry that is actually
/// there: `sum(F) = 1 - sky`.
///
/// The directions come from a Halton sequence, identical for every surfel and
/// rotated into its local frame, so this stays deterministic and adds no noise
/// — a stochastic estimate here would reintroduce exactly the sampling variance
/// the renderer exists to avoid.
pub fn hemisphere_closure(scene: &Scene, p: Vec3, n: Vec3, tmax: f32, dirs: u32) -> (f32, u32) {
    let (t, b) = crate::ltc::frame(n);
    let mut open = 0u32;
    let mut steps = 0u32;

    for k in 0..dirs {
        // Cosine-weighted hemisphere sampling, so the estimate is already
        // weighted the same way the form factors are.
        let u1 = radical_inverse(k + 1, 2);
        let u2 = radical_inverse(k + 1, 3);
        let r = u1.sqrt();
        let phi = std::f32::consts::TAU * u2;
        let d = t * (r * phi.cos()) + b * (r * phi.sin()) + n * (1.0 - u1).max(0.0).sqrt();

        let (hit, s) = trace(scene, p + n * 1e-3, d.normalize(), 1e-3, tmax);
        steps += s;
        if hit.is_none() {
            open += 1;
        }
    }
    (open as f32 / dirs as f32, steps)
}

#[inline]
fn radical_inverse(mut i: u32, base: u32) -> f32 {
    let mut f = 1.0f32;
    let mut r = 0.0f32;
    let inv = 1.0 / base as f32;
    while i > 0 {
        f *= inv;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene;

    /// A floor and a ceiling, with the light embedded flush in the ceiling —
    /// the exact geometry that broke the naive shadow cone.
    const FLUSH_CEILING_LIGHT: &str = r#"
render { width: 8, height: 8 }
camera { pos: [0,1,5], look: [0,1,0], fov: 40 }
material "w" { albedo: [0.8,0.8,0.8], roughness: 1.0 }
box { center: [0,-0.1,0], size: [8,0.2,8], mat: "w" }
box { center: [0,4.1,0], size: [8,0.2,8], mat: "w" }
light { verts: [[-0.7,3.94,-0.7],[0.7,3.94,-0.7],[0.7,3.94,0.7],[-0.7,3.94,0.7]], emit: [10,10,10] }
"#;

    /// Guards the shadow-cone bug. `h` is an *omnidirectional* distance: it
    /// reports the nearest surface in any direction, not just ahead. A shadow
    /// ray climbing toward a flush-mounted ceiling light runs nearly parallel to
    /// the ceiling plane, so `h` stays tiny and the naive `min(k*h/t)` drives
    /// visibility toward zero — darkening a floor that is in direct view of the
    /// light, with nothing between them.
    #[test]
    fn unoccluded_floor_under_flush_light_is_not_self_shadowed() {
        let sc = scene::parse(FLUSH_CEILING_LIGHT).expect("test scene parses");
        let light = sc.lights[0];
        let p = Vec3::ZERO;
        let n = Vec3::Y;

        let to = light.center() - p;
        let dist = to.length();
        let radius = (light.area() / std::f32::consts::PI).sqrt();
        let k = (dist / radius).clamp(1.5, 96.0);
        let end = (dist - radius * 1.25).max(0.05);

        let (vis, _) = soft_shadow(&sc, p + n * 3e-3, to / dist, 0.015, end, k);
        assert!(
            vis > 0.9,
            "floor directly beneath an unobstructed light is only {:.3} lit — \
             the shadow cone is reading the ceiling plane it runs parallel to",
            vis
        );
    }

    /// The complementary case: an occluder between surface and light must
    /// actually cast a shadow, so the fix above cannot be "always return 1".
    #[test]
    fn real_occluder_still_casts_shadow() {
        let mut src = FLUSH_CEILING_LIGHT.to_string();
        src.push_str("box { center: [0,2,0], size: [1.5,0.2,1.5], mat: \"w\" }\n");
        let sc = scene::parse(&src).expect("test scene parses");
        let light = sc.lights[0];

        let to = light.center() - Vec3::ZERO;
        let dist = to.length();
        let radius = (light.area() / std::f32::consts::PI).sqrt();
        let k = (dist / radius).clamp(1.5, 96.0);
        let end = (dist - radius * 1.25).max(0.05);

        let (vis, _) = soft_shadow(&sc, Vec3::new(0.0, 3e-3, 0.0), to / dist, 0.015, end, k);
        assert!(vis < 0.1, "occluded point reported {:.3} visibility", vis);
    }
}
