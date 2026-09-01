//! Signed distance primitives and CSG composition.
//!
//! Every primitive returns a true Euclidean distance (or a conservative
//! lower bound), which is what makes sphere tracing safe: stepping by the
//! distance value can never overshoot a surface.

use glam::Vec3;

#[derive(Clone, Copy, Debug)]
pub enum Prim {
    Sphere { c: Vec3, r: f32 },
    RoundBox { c: Vec3, b: Vec3, r: f32 },
    Plane { n: Vec3, h: f32 },
    Capsule { a: Vec3, b: Vec3, r: f32 },
    Torus { c: Vec3, major: f32, minor: f32 },
}

impl Prim {
    #[inline(always)]
    pub fn dist(&self, p: Vec3) -> f32 {
        match *self {
            Prim::Sphere { c, r } => (p - c).length() - r,

            // Inigo Quilez's rounded box: exact outside, conservative inside.
            Prim::RoundBox { c, b, r } => {
                let q = (p - c).abs() - (b - Vec3::splat(r));
                q.max(Vec3::ZERO).length() + q.x.max(q.y.max(q.z)).min(0.0) - r
            }

            Prim::Plane { n, h } => p.dot(n) - h,

            Prim::Capsule { a, b, r } => {
                let pa = p - a;
                let ba = b - a;
                let h = (pa.dot(ba) / ba.dot(ba)).clamp(0.0, 1.0);
                (pa - ba * h).length() - r
            }

            Prim::Torus { c, major, minor } => {
                let q = p - c;
                let xz = (q.x * q.x + q.z * q.z).sqrt() - major;
                (xz * xz + q.y * q.y).sqrt() - minor
            }
        }
    }

    /// Conservative AABB. Planes are unbounded and are excluded from the BVH.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        match *self {
            Prim::Sphere { c, r } => Some((c - Vec3::splat(r), c + Vec3::splat(r))),
            Prim::RoundBox { c, b, .. } => Some((c - b, c + b)),
            Prim::Plane { .. } => None,
            Prim::Capsule { a, b, r } => {
                Some((a.min(b) - Vec3::splat(r), a.max(b) + Vec3::splat(r)))
            }
            Prim::Torus { c, major, minor } => {
                let e = Vec3::new(major + minor, minor, major + minor);
                Some((c - e, c + e))
            }
        }
    }
}

/// One shaded entity: a base primitive with optional CSG subtractions.
///
/// Subtraction is kept *inside* the object rather than as a scene-level
/// operator, so the object's AABB stays valid and the BVH stays correct.
#[derive(Clone, Debug)]
pub struct Object {
    pub prim: Prim,
    pub subtract: Vec<Prim>,
    pub mat: u32,
    /// Smooth-union blend radius against neighbours in the same group. 0 = hard union.
    pub blend: f32,
}

impl Object {
    #[inline(always)]
    pub fn dist(&self, p: Vec3) -> f32 {
        let mut d = self.prim.dist(p);
        for s in &self.subtract {
            d = d.max(-s.dist(p));
        }
        d
    }
}
