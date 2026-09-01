//! BSDF evaluation and analytic direct lighting.

use crate::ltc;
use crate::scene::{Material, QuadLight, Scene};
use crate::trace;
use glam::Vec3;

/// Schlick's approximation to the Fresnel equations.
#[inline]
pub fn fresnel_schlick(cos_theta: f32, f0: Vec3) -> Vec3 {
    let m = (1.0 - cos_theta).clamp(0.0, 1.0);
    let m2 = m * m;
    f0 + (Vec3::ONE - f0) * (m2 * m2 * m)
}

/// Trowbridge-Reitz (GGX) normal distribution.
#[inline]
fn d_ggx(n_dot_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    a2 / (std::f32::consts::PI * d * d).max(1e-9)
}

/// Height-correlated Smith visibility, already divided by `4 nl nv`.
#[inline]
fn v_smith(n_dot_v: f32, n_dot_l: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let gv = n_dot_l * (n_dot_v * n_dot_v * (1.0 - a2) + a2).sqrt();
    let gl = n_dot_v * (n_dot_l * n_dot_l * (1.0 - a2) + a2).sqrt();
    0.5 / (gv + gl).max(1e-9)
}

/// Base reflectance at normal incidence. Dielectrics sit at 0.04; metals take
/// their albedo as F0 and have no diffuse lobe.
#[inline]
pub fn f0_of(m: &Material) -> Vec3 {
    Vec3::splat(0.04).lerp(m.albedo, m.metallic)
}

#[inline]
pub fn diffuse_albedo(m: &Material) -> Vec3 {
    m.albedo * (1.0 - m.metallic)
}

/// Analytic direct lighting from every area light in the scene.
///
/// Returns radiance and the number of SDF evaluations spent on shadow cones.
/// Nothing here is sampled stochastically, so this term is fully converged on
/// its first and only evaluation.
pub fn direct(scene: &Scene, p: Vec3, n: Vec3, v: Vec3, m: &Material) -> (Vec3, u32) {
    let mut out = Vec3::ZERO;
    let mut steps = 0u32;

    let (tan, bit) = ltc::frame(n);
    let n_dot_v = n.dot(v).max(1e-4);
    let alpha = (m.roughness * m.roughness).max(1e-3);
    let f0 = f0_of(m);
    let kd = diffuse_albedo(m);

    for light in &scene.lights {
        // Cull lights facing away from the shading point.
        let lc = light.center();
        let to_light = lc - p;
        if light.normal().dot(-to_light.normalize()) <= 0.0 {
            continue;
        }

        // --- exact diffuse: cosine-weighted solid angle of the polygon ---
        let local = ltc::to_local(&light.verts, p, tan, bit, n);
        let ff = ltc::quad_form_factor(&local);
        if ff <= 1e-6 {
            continue;
        }

        // --- visibility: one cone trace, analytic penumbra ---
        // The cone's tightness is the light's angular radius seen from p, so
        // large lights soften shadows exactly as they should.
        let dist = to_light.length();
        let radius = (light.area() / std::f32::consts::PI).sqrt().max(1e-4);
        let k = (dist / radius).clamp(1.5, 96.0);
        let ldir = to_light / dist;
        // Stop a full light-radius short: see the note on `soft_shadow`.
        let shadow_end = (dist - radius * 1.25).max(0.05);
        let (vis, s) = trace::soft_shadow(scene, p + n * 3e-3, ldir, 0.015, shadow_end, k);
        steps += s;
        if vis <= 1e-4 {
            continue;
        }

        let radiance = light.emit * vis;
        out += kd * radiance * ff;

        // --- specular: representative point on the light for the GGX lobe ---
        let r = (-v) - n * (2.0 * n.dot(-v));
        let plane_n = light.normal();
        let denom = r.dot(plane_n);
        let mut rep = lc;
        if denom.abs() > 1e-6 {
            let t = (light.verts[0] - p).dot(plane_n) / denom;
            if t > 0.0 {
                rep = ltc::closest_on_quad(&light.verts, p + r * t);
            }
        }
        let l = (rep - p).normalize();
        let n_dot_l = n.dot(l);
        if n_dot_l <= 0.0 {
            continue;
        }

        // Widen the lobe by the light's solid angle and renormalise so total
        // energy is preserved as the light grows (Karis 2013).
        let rep_dist = (rep - p).length().max(1e-4);
        let wide = (alpha + radius / (2.0 * rep_dist)).clamp(alpha, 1.0);
        let energy = (alpha / wide) * (alpha / wide);

        let h = (l + v).normalize();
        let d = d_ggx(n.dot(h).max(0.0), wide);
        let vis_s = v_smith(n_dot_v, n_dot_l, wide);
        let f = fresnel_schlick(l.dot(h).max(0.0), f0);

        out += f * (d * vis_s * energy * n_dot_l) * radiance;
    }

    (out, steps)
}

/// Diffuse-only direct lighting, used to seed the radiosity solve.
///
/// Shares the exact form factor and cone-shadow path with `direct`, minus the
/// specular lobe: specular is view-dependent, and baking it into a
/// view-independent surfel cache would smear a highlight across every camera
/// angle that later reads the cache.
pub fn direct_diffuse(scene: &Scene, p: Vec3, n: Vec3, albedo: Vec3) -> (Vec3, u32) {
    let mut out = Vec3::ZERO;
    let mut steps = 0u32;
    let (tan, bit) = ltc::frame(n);

    for light in &scene.lights {
        let lc = light.center();
        let to_light = lc - p;
        if light.normal().dot(-to_light.normalize()) <= 0.0 {
            continue;
        }

        let local = ltc::to_local(&light.verts, p, tan, bit, n);
        let ff = ltc::quad_form_factor(&local);
        if ff <= 1e-6 {
            continue;
        }

        let dist = to_light.length();
        let radius = (light.area() / std::f32::consts::PI).sqrt().max(1e-4);
        let k = (dist / radius).clamp(1.5, 96.0);
        let shadow_end = (dist - radius * 1.25).max(0.05);
        let (vis, s) =
            trace::soft_shadow(scene, p + n * 3e-3, to_light / dist, 0.015, shadow_end, k);
        steps += s;
        if vis <= 1e-4 {
            continue;
        }
        out += albedo * light.emit * (ff * vis);
    }
    (out, steps)
}

/// Radiance emitted by any light quad the ray hits directly, so emitters are
/// visible rather than black holes in the image.
pub fn emitter_hit(lights: &[QuadLight], ro: Vec3, rd: Vec3, tmax: f32) -> Option<(f32, Vec3)> {
    let mut best: Option<(f32, Vec3)> = None;
    for l in lights {
        if let Some(t) = trace::intersect_quad(&l.verts, ro, rd) {
            // Only the emitting face is visible. A back-facing emitter is
            // skipped rather than drawn black, so an off-camera fill light does
            // not punch a dark rectangle through the scene behind it.
            if t < tmax && l.normal().dot(rd) < 0.0 && best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, l.emit));
            }
        }
    }
    best
}
