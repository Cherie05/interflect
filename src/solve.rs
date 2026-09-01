//! The radiosity solve: Jacobi iteration on the cached transfer matrix.
//!
//! `B_i = E_i + rho_i * sum_c F_ic * B_c`
//!
//! The matrix from `formfactor` is fixed, so each bounce is a single dense
//! matrix-vector product. That is why bounce count is nearly free here while a
//! path tracer pays full price for every extra bounce: the expensive part
//! (visibility) was amortised into the matrix build.
//!
//! Jacobi rather than Gauss-Seidel, deliberately: Jacobi reads only the
//! previous iterate, so rows can be updated in parallel and in any order and
//! still produce identical results. Gauss-Seidel converges in fewer iterations
//! but its result depends on update order, which would forfeit the determinism
//! guarantee for a saving the amortised matrix already makes irrelevant.

use crate::formfactor::Transfer;
use crate::scene::Scene;
use crate::shade;
use crate::surfel::Surfel;
use glam::Vec3;
use rayon::prelude::*;

/// Direct lighting on every surfel — the `E` term.
///
/// Diffuse only: the specular lobe is view-dependent and belongs at shading
/// time, not baked into a view-independent cache.
pub fn light_surfels(scene: &Scene, surfels: &mut [Surfel]) -> u64 {
    let evals: Vec<u64> = surfels
        .par_iter_mut()
        .map(|s| {
            let (c, e) = shade::direct_diffuse(scene, s.p, s.n, s.albedo);
            s.direct = c;
            s.rad = c;
            s.irr = Vec3::ZERO;
            e as u64
        })
        .collect();
    evals.iter().sum()
}

pub struct SolveStats {
    pub iterations: u32,
    pub residual: f32,
}

/// Iterate to convergence (or `max_bounces`, whichever comes first).
pub fn solve(surfels: &mut [Surfel], t: &Transfer, max_bounces: u32) -> SolveStats {
    let nc = t.nc;
    if nc == 0 {
        return SolveStats {
            iterations: 0,
            residual: 0.0,
        };
    }

    let mut cluster_rad = vec![Vec3::ZERO; nc];
    let mut residual = 0.0f32;
    let mut iterations = 0u32;

    for it in 0..max_bounces {
        // --- aggregate surfel radiosity into cluster radiosity ---
        // Serial and area-weighted. Serial because it is a reduction over
        // floats: a parallel fold would sum in nondeterministic order and the
        // last bits would drift between runs. It is O(N) against an O(N*C)
        // gather, so it costs essentially nothing.
        for (c, out) in t.clusters.iter().zip(cluster_rad.iter_mut()) {
            let mut acc = Vec3::ZERO;
            for &m in &c.members {
                let s = &surfels[m as usize];
                acc += s.rad * s.area;
            }
            *out = acc / c.area.max(1e-12);
        }

        // --- gather: one dense mat-vec, rows independent ---
        let deltas: Vec<f32> = surfels
            .par_iter_mut()
            .enumerate()
            .map(|(i, s)| {
                let (a, b) = (t.starts[i] as usize, t.starts[i + 1] as usize);
                let mut irr = Vec3::ZERO;
                for k in a..b {
                    irr += cluster_rad[t.idx[k] as usize] * t.w[k];
                }
                let new_rad = s.direct + s.albedo * irr;
                let d = (new_rad - s.rad).abs().max_element();
                s.irr = irr;
                s.rad = new_rad;
                d
            })
            .collect();

        residual = deltas.iter().copied().fold(0.0f32, f32::max);
        iterations = it + 1;

        // Energy is strictly decreasing per bounce (albedo < 1), so once the
        // change falls below display precision further bounces cannot alter a
        // pixel.
        if residual < 1e-4 {
            break;
        }
    }

    SolveStats {
        iterations,
        residual,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{formfactor, scene, surfel};

    const CLOSED_BOX: &str = r#"
render { width: 8, height: 8 }
camera { pos: [0,2,3], look: [0,2,0], fov: 40 }
material "w" { albedo: [0.7,0.7,0.7], roughness: 1.0 }
box { center: [0,-0.1,0],  size: [4.4,0.2,4.4], mat: "w" }
box { center: [0,4.1,0],   size: [4.4,0.2,4.4], mat: "w" }
box { center: [0,2,-2.1],  size: [4.4,4.4,0.2], mat: "w" }
box { center: [0,2,2.1],   size: [4.4,4.4,0.2], mat: "w" }
box { center: [-2.1,2,0],  size: [0.2,4.4,4.4], mat: "w" }
box { center: [2.1,2,0],   size: [0.2,4.4,4.4], mat: "w" }
light { verts: [[-0.7,3.9,-0.7],[0.7,3.9,-0.7],[0.7,3.9,0.7],[-0.7,3.9,0.7]], emit: [15,15,15] }
"#;

    fn solved() -> Vec<surfel::Surfel> {
        let sc = scene::parse(CLOSED_BOX).expect("test scene parses");
        let mut surfels = surfel::generate(&sc, 3000);
        let clusters = formfactor::build_clusters(&sc, &surfels, 8);
        let (t, _) = formfactor::build(&sc, &surfels, clusters);
        light_surfels(&sc, &mut surfels);
        solve(&mut surfels, &t, 16);
        surfels
    }

    /// Guards the black-seam bug. Multiplying the solved irradiance by ambient
    /// occlusion double-counts visibility the transfer matrix already carries,
    /// and AO tends to zero exactly where two surfaces meet — while real global
    /// illumination gets *brighter* there, because the surfaces bounce into each
    /// other. The symptom was a black line along every wall junction.
    ///
    /// Inside a sealed lit box, no surfel that sees geometry may end up dark.
    #[test]
    fn no_interior_surfel_is_unlit() {
        let surfels = solved();
        let interior: Vec<_> = surfels
            .iter()
            .filter(|s| s.irr.max_element() > 0.0)
            .collect();
        assert!(
            interior.len() > surfels.len() / 4,
            "only {} of {} surfels received any indirect light",
            interior.len(),
            surfels.len()
        );

        let brightest = interior
            .iter()
            .map(|s| s.irr.max_element())
            .fold(0.0f32, f32::max);
        let dimmest = interior
            .iter()
            .map(|s| s.irr.max_element())
            .fold(f32::MAX, f32::min);
        assert!(
            dimmest > brightest * 1e-3,
            "dimmest lit surfel is {:.6} against a brightest of {:.6} — \
             a sealed box should have no near-black interior surface",
            dimmest,
            brightest
        );
    }

    /// Energy must strictly decrease per bounce for albedo < 1, so the solve has
    /// to converge rather than oscillate or diverge.
    #[test]
    fn solve_converges() {
        let sc = scene::parse(CLOSED_BOX).expect("test scene parses");
        let mut surfels = surfel::generate(&sc, 3000);
        let clusters = formfactor::build_clusters(&sc, &surfels, 8);
        let (t, _) = formfactor::build(&sc, &surfels, clusters);
        light_surfels(&sc, &mut surfels);
        let stats = solve(&mut surfels, &t, 64);
        assert!(
            stats.residual < 1e-3,
            "residual {:.2e} after {} bounces — solve is not converging",
            stats.residual,
            stats.iterations
        );
        assert!(
            stats.iterations < 64,
            "hit the bounce cap without converging"
        );
    }
}
