//! Form-factor transfer matrix between surfels and surfel clusters.
//!
//! The radiosity equation needs the fraction of energy leaving patch `j` that
//! arrives at patch `i`. Doing that for every surfel pair is O(N^2) — at 16k
//! surfels that is 2.6e8 visibility queries, which is not viable.
//!
//! Instead, receivers stay at full surfel resolution while *emitters* are
//! aggregated into clusters: a coarse voxel grid crossed with the six major
//! normal directions. Splitting by normal matters — a voxel spanning a floor
//! and a wall has an area-weighted mean normal near zero and would radiate in
//! no direction at all. The result is a few hundred emitters instead of 16k,
//! making the matrix O(N * C) and small enough to hold densely.
//!
//! The matrix is built **once**. Every bounce afterwards is a matrix-vector
//! product over cached coefficients, so the expensive part — visibility — is
//! paid a single time no matter how many bounces the solve runs.

use crate::scene::Scene;
use crate::surfel::Surfel;
use crate::trace;
use glam::Vec3;
use rayon::prelude::*;
use std::collections::HashMap;

/// Ceiling on the energy-calibration rescale. A surfel wedged into a concave
/// corner can legitimately have a small raw sum, but an unbounded correction
/// there turns a geometry artefact into a blown-out highlight.
const MAX_CALIBRATION: f32 = 8.0;

/// Aggregated emitter: a group of surfels sharing a voxel and a normal octant.
#[derive(Clone, Debug)]
pub struct Cluster {
    pub p: Vec3,
    pub n: Vec3,
    pub area: f32,
    /// Radius of a disc with the cluster's area; sets the visibility cone width.
    pub radius: f32,
    pub members: Vec<u32>,
}

/// Sparse transfer matrix in CSR form.
///
/// Dense storage was the obvious first cut and does not survive contact with a
/// real scene: 24k surfels against 2.1k clusters is 208 MB, and most of it is
/// zero because the majority of cluster pairs are backfacing or occluded. CSR
/// holds only the surviving links at 8 bytes each, and the solve then iterates
/// exactly the nonzeros instead of scanning past them.
pub struct Transfer {
    pub clusters: Vec<Cluster>,
    /// Mean form-factor row sum, averaged over rows that have at least one
    /// link. In a closed room this must approach 1.0: every direction from an
    /// interior surface hits something. Anything well below that means the
    /// cluster set is failing to tile the hemisphere, and the scene renders too
    /// dark and tinted toward whichever clusters survived culling.
    ///
    /// Rows with no links are excluded deliberately. Surfels land on the
    /// *outside* of wall slabs too, and those correctly see nothing; averaging
    /// them in halves the figure and hides the number that matters.
    pub mean_row_sum: f32,
    /// Row `i` occupies `starts[i]..starts[i + 1]`.
    pub starts: Vec<u32>,
    pub idx: Vec<u32>,
    pub w: Vec<f32>,
    pub nc: usize,
    pub links: usize,
}

/// Index of the dominant axis-aligned direction of `n`, in 0..6.
#[inline]
fn normal_bucket(n: Vec3) -> usize {
    let a = n.abs();
    if a.x >= a.y && a.x >= a.z {
        if n.x >= 0.0 {
            0
        } else {
            1
        }
    } else if a.y >= a.z {
        if n.y >= 0.0 {
            2
        } else {
            3
        }
    } else if n.z >= 0.0 {
        4
    } else {
        5
    }
}

pub fn build_clusters(scene: &Scene, surfels: &[Surfel], res: i32) -> Vec<Cluster> {
    let (lo, hi) = scene.bounds;
    let ext = hi - lo;
    let inv = Vec3::new(
        res as f32 / ext.x.max(1e-6),
        res as f32 / ext.y.max(1e-6),
        res as f32 / ext.z.max(1e-6),
    );

    let mut map: HashMap<(i32, i32, i32, usize), Vec<u32>> = HashMap::new();
    for (i, s) in surfels.iter().enumerate() {
        let c = (s.p - lo) * inv;
        let key = (
            (c.x as i32).clamp(0, res - 1),
            (c.y as i32).clamp(0, res - 1),
            (c.z as i32).clamp(0, res - 1),
            normal_bucket(s.n),
        );
        map.entry(key).or_default().push(i as u32);
    }

    // HashMap iteration order is unspecified, so the cluster list is sorted by
    // key before use. Without this the cluster indices — and therefore the
    // float summation order in the solve — would vary between runs.
    let mut keys: Vec<_> = map.keys().copied().collect();
    keys.sort_unstable();

    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        let members = map.remove(&k).unwrap();
        let mut p = Vec3::ZERO;
        let mut n = Vec3::ZERO;
        let mut area = 0.0f32;
        for &m in &members {
            let s = &surfels[m as usize];
            p += s.p * s.area;
            n += s.n * s.area;
            area += s.area;
        }
        if area <= 1e-12 {
            continue;
        }
        let n = if n.length_squared() > 1e-12 {
            n.normalize()
        } else {
            Vec3::Y
        };
        out.push(Cluster {
            p: p / area,
            n,
            area,
            radius: (area / std::f32::consts::PI).sqrt(),
            members,
        });
    }
    out
}

/// Build the surfel-to-cluster transfer matrix.
///
/// Rows are independent, so this parallelises without any cross-thread
/// accumulation — the matrix is bit-identical regardless of thread count.
pub fn build(scene: &Scene, surfels: &[Surfel], clusters: Vec<Cluster>) -> (Transfer, u64) {
    let nc = clusters.len();
    let diag = (scene.bounds.1 - scene.bounds.0).length();

    // Rows are fully independent, so this parallelises with no cross-thread
    // accumulation. `collect` preserves index order, so concatenating the rows
    // afterwards yields the same matrix for any thread count.
    let rows: Vec<(Vec<u32>, Vec<f32>, u64, f32)> = surfels
        .par_iter()
        .map(|s| {
            let mut idx: Vec<u32> = Vec::new();
            let mut wt: Vec<f32> = Vec::new();
            let mut evals = 0u64;
            let mut sum = 0.0f32;

            for (ci, c) in clusters.iter().enumerate() {
                let d = c.p - s.p;
                let r2 = d.length_squared();
                if r2 < 1e-9 {
                    continue;
                }
                let r = r2.sqrt();
                let dir = d / r;

                let cos_i = s.n.dot(dir);
                let cos_c = c.n.dot(-dir);
                if cos_i <= 1e-3 || cos_c <= 1e-3 {
                    continue;
                }

                // Nusselt-analog disc-to-disc form factor. The `+ area` term in
                // the denominator removes the r -> 0 singularity that makes the
                // naive point form factor explode for adjacent patches.
                let ff = cos_i * cos_c * c.area / (std::f32::consts::PI * r2 + c.area);
                // Below this a link cannot move a pixel, and skipping it avoids
                // the visibility trace — which is essentially the entire cost of
                // this function.
                if ff < 2e-5 {
                    continue;
                }

                // Visibility as a cone rather than a binary test: a cluster is
                // spatially extended, so partial occlusion is the common case
                // and a hard shadow ray would quantise it into blocky patches.
                let k = (r / c.radius.max(1e-4)).clamp(1.5, 32.0);
                let (vis, e) = trace::soft_shadow(
                    scene,
                    s.p + s.n * (diag * 5e-4),
                    dir,
                    diag * 1e-3,
                    r - c.radius * 0.75,
                    k,
                );
                evals += e as u64;
                if vis <= 1e-3 {
                    continue;
                }

                let v = ff * vis;
                idx.push(ci as u32);
                wt.push(v);
                sum += v;
            }

            // --- energy calibration ---
            // Rescale the row so its total matches the fraction of the
            // hemisphere that actually contains geometry. Two errors are
            // corrected at once: surfel areas are inferred from an assumed
            // Poisson-disc packing density and carry a few percent of error,
            // and the point-to-disc form factor underestimates any cluster
            // subtending a large solid angle. Measured row sums were 0.213 in a
            // closed box whose true value is 1.0 — a scene rendered nearly five
            // times too dark. Capping at 1.0 also keeps the Jacobi iteration
            // unconditionally convergent for any albedo below 1.
            let (sky, e) = trace::hemisphere_closure(scene, s.p, s.n, diag * 2.0, 16);
            evals += e as u64;
            let target = (1.0 - sky).clamp(0.0, 1.0);
            if sum > 1e-5 {
                let scale = (target / sum).min(MAX_CALIBRATION);
                for v in wt.iter_mut() {
                    *v *= scale;
                }
                sum *= scale;
            }

            (idx, wt, evals, sum.min(1.0))
        })
        .collect();

    let links: usize = rows.iter().map(|r| r.0.len()).sum();
    let evals: u64 = rows.iter().map(|r| r.2).sum();
    let interior = rows.iter().filter(|r| !r.0.is_empty()).count();
    let mean_row_sum = if interior == 0 {
        0.0
    } else {
        rows.iter()
            .filter(|r| !r.0.is_empty())
            .map(|r| r.3)
            .sum::<f32>()
            / interior as f32
    };

    let mut starts = Vec::with_capacity(surfels.len() + 1);
    let mut idx = Vec::with_capacity(links);
    let mut w = Vec::with_capacity(links);
    starts.push(0u32);
    for (ri, rw, _, _) in rows {
        idx.extend_from_slice(&ri);
        w.extend_from_slice(&rw);
        starts.push(idx.len() as u32);
    }

    (
        Transfer {
            clusters,
            mean_row_sum,
            starts,
            idx,
            w,
            nc,
            links,
        },
        evals,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scene, surfel};

    /// Fully enclosed box: six slabs, no opening. Every interior surfel sees
    /// geometry in every direction, so its form factors must sum to ~1.
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

    fn build_transfer(n: usize) -> (Vec<surfel::Surfel>, Transfer) {
        let sc = scene::parse(CLOSED_BOX).expect("test scene parses");
        let surfels = surfel::generate(&sc, n);
        assert!(!surfels.is_empty(), "no surfels generated");
        let clusters = build_clusters(&sc, &surfels, 8);
        let (t, _) = build(&sc, &surfels, clusters);
        (surfels, t)
    }

    /// Guards the energy-conservation bug. Raw point-to-disc form factors
    /// underestimate any cluster subtending a large solid angle: measured row
    /// sums were 0.213 inside a closed box whose true value is exactly 1.0,
    /// leaving the scene roughly five times too dark. The hemisphere-closure
    /// calibration is what restores it.
    #[test]
    fn closed_box_conserves_energy() {
        let (_, t) = build_transfer(3000);
        assert!(
            t.mean_row_sum > 0.7,
            "interior row sums average {:.3}; inside a sealed box every direction \
             hits a surface, so this must approach 1.0",
            t.mean_row_sum
        );
        assert!(
            t.mean_row_sum <= 1.0001,
            "row sums exceed 1.0 ({:.3}) — a surface cannot receive more than \
             all incident energy, and the solve would diverge",
            t.mean_row_sum
        );
    }

    /// Guards the dense-matrix blowup. Dense storage was 208 MB for a real
    /// scene and most of it was structural zeros.
    #[test]
    fn transfer_matrix_is_sparse_and_well_formed() {
        let (surfels, t) = build_transfer(3000);

        assert_eq!(
            t.starts.len(),
            surfels.len() + 1,
            "CSR row offsets malformed"
        );
        assert_eq!(t.idx.len(), t.w.len());
        assert_eq!(*t.starts.last().unwrap() as usize, t.idx.len());

        for w in t.starts.windows(2) {
            assert!(w[1] >= w[0], "CSR offsets must be non-decreasing");
        }
        for &c in &t.idx {
            assert!((c as usize) < t.nc, "cluster index {} out of range", c);
        }

        let dense = surfels.len() * t.nc;
        assert!(
            t.links * 3 < dense,
            "{} links against {} dense cells — matrix is not sparse, \
             dense storage would be {} MB",
            t.links,
            dense,
            dense * 4 / 1_048_576
        );
    }
}
