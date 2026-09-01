//! Meshless surface sampling on a signed distance field.
//!
//! This is the step that makes the whole approach work. Classical radiosity
//! (Goral et al., 1984) produced noise-free global illumination but lost to path
//! tracing for one reason: it needed the scene subdivided into well-conditioned
//! patches, and automatic meshing was fragile, slow, and artefact-prone.
//!
//! An SDF has no mesh to subdivide. The surface is the zero level set, and any
//! point in space can be projected onto it by Newton iteration along the
//! gradient: `p <- p - f(p) * grad f(p)`, which converges in a handful of steps
//! because `|grad f| = 1` for a true distance field. So the patches can be
//! generated directly, at any density, with no topology, no seams, and no
//! failure cases — the obstacle that killed radiosity simply is not present.
//!
//! Seeding uses a Halton sequence rather than an RNG, so the surfel set is a
//! pure function of the scene and the requested count. Nothing here is random,
//! and the output is bit-identical across runs and thread counts.

use crate::scene::Scene;
use glam::Vec3;

#[derive(Clone, Copy, Debug)]
pub struct Surfel {
    pub p: Vec3,
    pub n: Vec3,
    pub albedo: Vec3,
    pub area: f32,
    /// Outgoing diffuse radiance from *direct* light. The `E` term.
    pub direct: Vec3,
    /// Incident indirect irradiance gathered from other surfels. This is what
    /// shading interpolates: it is incoming light, so the receiving surface
    /// applies its own albedo to it. Interpolating outgoing radiosity instead
    /// would double-apply albedo and tint every surface with its neighbours.
    pub irr: Vec3,
    /// Total outgoing radiosity `B = direct + albedo * irr`. Only the solve and
    /// the cluster aggregation read this.
    pub rad: Vec3,
}

/// Radical-inverse Halton sequence. Deterministic, low-discrepancy, and
/// stratified in every projection — which is what keeps surfel spacing even
/// without any rejection bias.
#[inline]
fn halton(mut i: u32, base: u32) -> f32 {
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

/// Uniform spatial hash grid over the scene bounds.
///
/// Backed by a flat `Vec` rather than a `HashMap`: iteration order over a
/// `HashMap` is unspecified, and any order-dependent float accumulation would
/// break the determinism guarantee.
pub struct Grid {
    pub lo: Vec3,
    pub inv_cell: f32,
    pub dim: [i32; 3],
    /// CSR-style: `starts[c]..starts[c+1]` indexes into `items`.
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl Grid {
    pub fn build(points: &[Vec3], lo: Vec3, hi: Vec3, cell: f32) -> Grid {
        let inv_cell = 1.0 / cell;
        let ext = hi - lo;
        let dim = [
            ((ext.x * inv_cell).ceil() as i32).clamp(1, 512),
            ((ext.y * inv_cell).ceil() as i32).clamp(1, 512),
            ((ext.z * inv_cell).ceil() as i32).clamp(1, 512),
        ];
        let ncells = (dim[0] * dim[1] * dim[2]) as usize;

        let idx_of = |p: Vec3| -> usize {
            let c = [
                (((p.x - lo.x) * inv_cell) as i32).clamp(0, dim[0] - 1),
                (((p.y - lo.y) * inv_cell) as i32).clamp(0, dim[1] - 1),
                (((p.z - lo.z) * inv_cell) as i32).clamp(0, dim[2] - 1),
            ];
            (c[0] + dim[0] * (c[1] + dim[1] * c[2])) as usize
        };

        // Counting sort: two passes, no per-cell allocation.
        let mut counts = vec![0u32; ncells + 1];
        for &p in points {
            counts[idx_of(p) + 1] += 1;
        }
        for i in 0..ncells {
            counts[i + 1] += counts[i];
        }
        let starts = counts.clone();
        let mut cursor = counts;
        let mut items = vec![0u32; points.len()];
        for (i, &p) in points.iter().enumerate() {
            let c = idx_of(p);
            items[cursor[c] as usize] = i as u32;
            cursor[c] += 1;
        }

        Grid {
            lo,
            inv_cell,
            dim,
            starts,
            items,
        }
    }

    #[inline]
    fn cell_of(&self, p: Vec3) -> [i32; 3] {
        [
            (((p.x - self.lo.x) * self.inv_cell) as i32).clamp(0, self.dim[0] - 1),
            (((p.y - self.lo.y) * self.inv_cell) as i32).clamp(0, self.dim[1] - 1),
            (((p.z - self.lo.z) * self.inv_cell) as i32).clamp(0, self.dim[2] - 1),
        ]
    }

    /// Visit every stored index within `r` cells of `p`, in deterministic order.
    #[inline]
    pub fn for_each_near<F: FnMut(u32)>(&self, p: Vec3, r: i32, mut f: F) {
        let c = self.cell_of(p);
        for z in (c[2] - r).max(0)..=(c[2] + r).min(self.dim[2] - 1) {
            for y in (c[1] - r).max(0)..=(c[1] + r).min(self.dim[1] - 1) {
                for x in (c[0] - r).max(0)..=(c[0] + r).min(self.dim[0] - 1) {
                    let ci = (x + self.dim[0] * (y + self.dim[1] * z)) as usize;
                    let (s, e) = (self.starts[ci] as usize, self.starts[ci + 1] as usize);
                    for &it in &self.items[s..e] {
                        f(it);
                    }
                }
            }
        }
    }
}

/// Generate surfels by projecting a Halton point set onto the SDF surface.
///
/// `target` is the requested count; the achieved count differs because
/// Poisson-style rejection depends on how much surface the scene actually has.
pub fn generate(scene: &Scene, target: usize) -> Vec<Surfel> {
    let (lo, hi) = scene.bounds;
    let diag = (hi - lo).length();

    // Minimum separation that would place `target` points on a surface whose
    // area scales with the bounding volume. Poisson-disc packing reaches about
    // 0.65 coverage, which is folded into the area estimate below.
    let d_min = (diag / (target as f32).sqrt()).max(1e-4);
    let cell = d_min;

    // Provisional grid sized for the target count; rebuilt as points accumulate.
    let mut pts: Vec<Vec3> = Vec::with_capacity(target);
    let mut out: Vec<Surfel> = Vec::with_capacity(target);

    // Coarse acceleration: a fixed-size occupancy grid holding the index of
    // accepted surfels, so rejection is O(1) rather than O(n).
    let inv_cell = 1.0 / cell;
    let dim = [
        (((hi.x - lo.x) * inv_cell).ceil() as i32).clamp(1, 512),
        (((hi.y - lo.y) * inv_cell).ceil() as i32).clamp(1, 512),
        (((hi.z - lo.z) * inv_cell).ceil() as i32).clamp(1, 512),
    ];
    let ncells = (dim[0] * dim[1] * dim[2]) as usize;
    let mut occ: Vec<Vec<u32>> = vec![Vec::new(); ncells];
    let cell_idx = |p: Vec3| -> [i32; 3] {
        [
            (((p.x - lo.x) * inv_cell) as i32).clamp(0, dim[0] - 1),
            (((p.y - lo.y) * inv_cell) as i32).clamp(0, dim[1] - 1),
            (((p.z - lo.z) * inv_cell) as i32).clamp(0, dim[2] - 1),
        ]
    };

    let eps = d_min * 0.02;
    let d_min_sq = d_min * d_min;
    // Oversample: most seeds land on a surface already occupied by an earlier
    // surfel and get rejected.
    let seeds = (target as u32).saturating_mul(24).min(6_000_000);

    for i in 0..seeds {
        if out.len() >= target {
            break;
        }

        // Halton in 3D (bases 2, 3, 5) across the scene bounds.
        let s = Vec3::new(halton(i + 1, 2), halton(i + 1, 3), halton(i + 1, 5));
        let mut p = lo + (hi - lo) * s;

        // Newton projection onto the zero level set.
        let mut ok = false;
        for _ in 0..10 {
            let d = scene.dist(p);
            if d.abs() < eps {
                ok = true;
                break;
            }
            if !d.is_finite() || d.abs() > diag {
                break;
            }
            p -= scene.normal(p) * d;
        }
        if !ok || scene.dist(p).abs() >= eps {
            continue;
        }
        if p.cmplt(lo).any() || p.cmpgt(hi).any() {
            continue;
        }

        // Poisson-disc rejection against already-accepted surfels.
        let c = cell_idx(p);
        let mut too_close = false;
        'outer: for z in (c[2] - 1).max(0)..=(c[2] + 1).min(dim[2] - 1) {
            for y in (c[1] - 1).max(0)..=(c[1] + 1).min(dim[1] - 1) {
                for x in (c[0] - 1).max(0)..=(c[0] + 1).min(dim[0] - 1) {
                    let ci = (x + dim[0] * (y + dim[1] * z)) as usize;
                    for &j in &occ[ci] {
                        if (pts[j as usize] - p).length_squared() < d_min_sq {
                            too_close = true;
                            break 'outer;
                        }
                    }
                }
            }
        }
        if too_close {
            continue;
        }

        let hit = scene.dist_mat(p);
        let m = scene.material(hit.mat);
        let n = scene.normal(p);

        let ci = (c[0] + dim[0] * (c[1] + dim[1] * c[2])) as usize;
        occ[ci].push(out.len() as u32);
        pts.push(p);
        out.push(Surfel {
            p,
            n,
            albedo: crate::shade::diffuse_albedo(&m),
            // Poisson-disc packing covers ~0.65 of the plane at separation
            // d_min, so each disc stands in for d_min^2 / 0.65 of surface.
            area: d_min * d_min / 0.65,
            direct: Vec3::ZERO,
            irr: Vec3::ZERO,
            rad: Vec3::ZERO,
        });
    }

    out
}

/// Interpolate cached indirect irradiance at an arbitrary shading point.
///
/// This is the reason the solve is view-independent: the lighting lives on the
/// surface, not in the image, so a new camera costs only this lookup — no
/// re-solve, no new rays.
pub fn gather(surfels: &[Surfel], grid: &Grid, p: Vec3, n: Vec3, radius: f32) -> Vec3 {
    let mut acc = Vec3::ZERO;
    let mut wsum = 0.0f32;
    let r2 = radius * radius;

    // Cell size equals the gather radius, so a 3x3x3 block is guaranteed to
    // contain every surfel within range.
    grid.for_each_near(p, 1, |i| {
        let s = &surfels[i as usize];
        let d2 = (s.p - p).length_squared();
        if d2 > r2 {
            return;
        }
        // Reject surfels facing away: they belong to a different surface, and
        // blending them in is what causes light to leak through thin walls.
        let align = s.n.dot(n);
        if align <= 0.15 {
            return;
        }
        // Smooth falloff so the interpolation has no visible cell structure.
        let t = 1.0 - d2 / r2;
        let w = t * t * align;
        acc += s.irr * w;
        wsum += w;
    });

    if wsum > 1e-6 {
        acc / wsum
    } else {
        Vec3::ZERO
    }
}
