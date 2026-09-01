//! Brute-force Monte Carlo path tracer.
//!
//! This is *not* part of the renderer. It exists solely as ground truth: an
//! accuracy claim is meaningless without something correct to measure against,
//! and shipping the reference in the same binary means anyone can reproduce the
//! benchmark table without installing PBRT or Blender.
//!
//! It is deliberately naive — uniform light sampling, cosine-weighted indirect,
//! Russian roulette, no importance sampling beyond that — because a simple
//! integrator is easier to verify by inspection than a fast one. At high sample
//! counts it converges to the true solution of the rendering equation, which is
//! all that is required of it.
//!
//! Comparisons run on Lambertian scenes. That isolates what is actually being
//! validated: whether the surfel radiosity solve transports diffuse energy
//! correctly. The specular path is textbook GGX shared by both, and folding it
//! in would measure agreement between two copies of the same code.

use crate::scene::Scene;
use crate::shade;
use crate::trace;
use glam::Vec3;

/// PCG32. Deterministic given the seed, so the reference is reproducible too.
pub struct Rng(u64);

impl Rng {
    /// Identity helper so call sites read cleanly.
    pub fn into_seeded(self) -> Rng {
        self
    }
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(6364136223846793005).wrapping_add(1))
    }
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let x = ((self.0 >> 18) ^ self.0) >> 27;
        let rot = (self.0 >> 59) as u32;
        let v = (x as u32).rotate_right(rot);
        // 24 bits of mantissa is ample and avoids ever returning exactly 1.0.
        (v >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}

#[inline]
fn cosine_hemisphere(n: Vec3, rng: &mut Rng) -> Vec3 {
    let (t, b) = crate::ltc::frame(n);
    let u1 = rng.next_f32();
    let u2 = rng.next_f32();
    let r = u1.sqrt();
    let phi = std::f32::consts::TAU * u2;
    (t * (r * phi.cos()) + b * (r * phi.sin()) + n * (1.0 - u1).max(0.0).sqrt()).normalize()
}

/// One path. Lambertian surfaces, next-event estimation against every quad
/// light, cosine-weighted indirect bounces, Russian roulette after depth 3.
pub fn path(sc: &Scene, ro: Vec3, rd: Vec3, tmax: f32, max_depth: u32, rng: &mut Rng) -> Vec3 {
    let mut radiance = Vec3::ZERO;
    let mut throughput = Vec3::ONE;
    let mut origin = ro;
    let mut dir = rd;

    for depth in 0..max_depth {
        let (hit, _) = trace::trace(sc, origin, dir, 1e-3, tmax);

        // An emitter seen directly. Only counted on the camera ray: at deeper
        // bounces the same energy already arrived through next-event
        // estimation, and adding both double-counts every light path.
        let surf_t = hit.map_or(f32::MAX, |h| h.t);
        if let Some((_, e)) = shade::emitter_hit(&sc.lights, origin, dir, surf_t.min(tmax)) {
            if depth == 0 {
                radiance += throughput * e;
            }
            break;
        }

        let h = match hit {
            Some(h) => h,
            None => break,
        };
        let m = sc.material(h.mat);
        let albedo = shade::diffuse_albedo(&m);
        radiance += throughput * m.emissive;

        // --- next event estimation ---
        for light in &sc.lights {
            let e1 = light.verts[1] - light.verts[0];
            let e2 = light.verts[3] - light.verts[0];
            let lp = light.verts[0] + e1 * rng.next_f32() + e2 * rng.next_f32();

            let to = lp - h.p;
            let dist2 = to.length_squared();
            if dist2 < 1e-9 {
                continue;
            }
            let dist = dist2.sqrt();
            let l = to / dist;

            let cos_s = h.n.dot(l);
            let cos_l = light.normal().dot(-l);
            if cos_s <= 0.0 || cos_l <= 0.0 {
                continue;
            }

            // Hard visibility: a reference must not soften anything.
            let (occ, _) = trace::trace(sc, h.p + h.n * 1e-3, l, 1e-3, dist - 1e-2);
            if occ.is_some() {
                continue;
            }

            // Area-to-solid-angle conversion for the uniform area sample.
            let pdf = dist2 / (cos_l * light.area());
            radiance += throughput * albedo * light.emit * (cos_s / (std::f32::consts::PI * pdf));
        }

        // --- indirect bounce ---
        // Cosine-weighted sampling cancels the cos/pdf factor exactly, leaving
        // the albedo alone.
        throughput *= albedo;
        dir = cosine_hemisphere(h.n, rng);
        origin = h.p + h.n * 1e-3;

        if depth >= 3 {
            let q = throughput.max_element().clamp(0.05, 0.95);
            if rng.next_f32() > q {
                break;
            }
            throughput /= q;
        }
    }

    radiance
}
