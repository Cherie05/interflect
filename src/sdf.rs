//! Signed distance primitives and CSG composition.
//!
//! Every primitive returns a true Euclidean distance (or a conservative
//! lower bound), which is what makes sphere tracing safe: stepping by the
//! distance value can never overshoot a surface.

use glam::Vec3;

#[derive(Clone, Copy, Debug)]
pub enum Prim {
    Sphere {
        c: Vec3,
        r: f32,
    },
    RoundBox {
        c: Vec3,
        b: Vec3,
        r: f32,
    },
    Plane {
        n: Vec3,
        h: f32,
    },
    Capsule {
        a: Vec3,
        b: Vec3,
        r: f32,
    },
    Torus {
        c: Vec3,
        major: f32,
        minor: f32,
    },
    /// Flat-capped cylinder between two points.
    Cylinder {
        a: Vec3,
        b: Vec3,
        r: f32,
    },
    /// Flat-capped cone; `ra` is the radius at `a`, `rb` at `b`. Setting `rb`
    /// to zero gives a true point-tipped cone.
    Cone {
        a: Vec3,
        b: Vec3,
        ra: f32,
        rb: f32,
    },
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

            // Quilez's capped cylinder. Unlike a capsule the ends are flat
            // discs, which is what you want for legs, columns and pins.
            Prim::Cylinder { a, b, r } => {
                let ba = b - a;
                let pa = p - a;
                let baba = ba.dot(ba);
                if baba < 1e-12 {
                    return (p - a).length() - r;
                }
                let paba = pa.dot(ba);
                let x = (pa * baba - ba * paba).length() - r * baba;
                let y = (paba - baba * 0.5).abs() - baba * 0.5;
                let x2 = x * x;
                let y2 = y * y * baba;
                let d = if x.max(y) < 0.0 {
                    -x2.min(y2)
                } else {
                    (if x > 0.0 { x2 } else { 0.0 }) + (if y > 0.0 { y2 } else { 0.0 })
                };
                d.signum() * d.abs().sqrt() / baba
            }

            // Quilez's capped cone. The two-radius form covers cones, truncated
            // cones and tapered columns with one primitive.
            Prim::Cone { a, b, ra, rb } => {
                let ba = b - a;
                let baba = ba.dot(ba);
                if baba < 1e-12 {
                    return (p - a).length() - ra.max(rb);
                }
                let rba = rb - ra;
                let pa = p - a;
                let papa = pa.dot(pa);
                let paba = pa.dot(ba) / baba;
                // Radial distance from the axis.
                let x = (papa - paba * paba * baba).max(0.0).sqrt();
                let cax = 0.0f32.max(x - if paba < 0.5 { ra } else { rb });
                let cay = (paba - 0.5).abs() - 0.5;
                let k = rba * rba + baba;
                let f = ((rba * (x - ra) + paba * baba) / k).clamp(0.0, 1.0);
                let cbx = x - ra - f * rba;
                let cby = paba - f;
                let s = if cbx < 0.0 && cay < 0.0 { -1.0 } else { 1.0 };
                s * (cax * cax + cay * cay * baba)
                    .min(cbx * cbx + cby * cby * baba)
                    .sqrt()
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
            Prim::Cylinder { a, b, r } => {
                Some((a.min(b) - Vec3::splat(r), a.max(b) + Vec3::splat(r)))
            }
            Prim::Cone { a, b, ra, rb } => {
                let r = ra.max(rb);
                Some((a.min(b) - Vec3::splat(r), a.max(b) + Vec3::splat(r)))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A distance field that lies produces subtly wrong images rather than an
    /// obvious failure, so every primitive is checked against distances worked
    /// out by hand.
    #[test]
    fn cylinder_distances_are_correct() {
        // Unit-radius cylinder along Y, from y=0 to y=2.
        let c = Prim::Cylinder {
            a: Vec3::new(0.0, 0.0, 0.0),
            b: Vec3::new(0.0, 2.0, 0.0),
            r: 1.0,
        };
        let cases = [
            // (point, expected distance)
            (Vec3::new(3.0, 1.0, 0.0), 2.0),  // beside it, radially
            (Vec3::new(0.0, 5.0, 0.0), 3.0),  // straight above the top cap
            (Vec3::new(0.0, -1.0, 0.0), 1.0), // below the bottom cap
            (Vec3::new(0.0, 1.0, 0.0), -1.0), // dead centre, inside
            (Vec3::new(1.0, 1.0, 0.0), 0.0),  // on the curved surface
            (Vec3::new(0.0, 2.0, 0.0), 0.0),  // on the top cap
        ];
        for (p, want) in cases {
            let got = c.dist(p);
            assert!(
                (got - want).abs() < 1e-4,
                "cylinder at {:?}: got {}, want {}",
                p,
                got,
                want
            );
        }
        // Flat caps, not rounded: a capsule would report sqrt(2) - 1 here,
        // a cylinder reports the corner distance.
        let corner = c.dist(Vec3::new(2.0, 3.0, 0.0));
        assert!(
            (corner - 2.0f32.sqrt()).abs() < 1e-4,
            "cap corner should be sqrt(2) away, got {}",
            corner
        );
    }

    #[test]
    fn cone_distances_are_correct() {
        // Point-tipped cone: radius 1 at y=0, tapering to nothing at y=2.
        let c = Prim::Cone {
            a: Vec3::new(0.0, 0.0, 0.0),
            b: Vec3::new(0.0, 2.0, 0.0),
            ra: 1.0,
            rb: 0.0,
        };
        assert!(
            c.dist(Vec3::new(0.0, 1.0, 0.0)) < 0.0,
            "axis should be inside"
        );
        assert!(
            c.dist(Vec3::new(0.0, 3.0, 0.0)) > 0.9,
            "above the tip is outside"
        );
        assert!(
            c.dist(Vec3::new(5.0, 1.0, 0.0)) > 3.5,
            "far to the side is outside"
        );
        // The base disc has radius 1, so this point sits exactly on its rim.
        assert!(
            c.dist(Vec3::new(1.0, 0.0, 0.0)).abs() < 1e-3,
            "base rim should be on the surface"
        );
        // Radius shrinks with height, so a point at r=0.9 is inside low down
        // and outside near the tip.
        assert!(c.dist(Vec3::new(0.9, 0.1, 0.0)) < 0.0);
        assert!(c.dist(Vec3::new(0.9, 1.8, 0.0)) > 0.0);
    }

    /// Sphere tracing only stays safe while a primitive never over-reports its
    /// distance. A field that claims more clearance than it has lets rays step
    /// straight through the surface.
    #[test]
    fn new_primitives_never_overstate_distance() {
        let prims = [
            Prim::Cylinder {
                a: Vec3::new(-0.4, 0.1, 0.2),
                b: Vec3::new(0.5, 1.3, -0.3),
                r: 0.35,
            },
            Prim::Cone {
                a: Vec3::new(0.2, 0.0, 0.1),
                b: Vec3::new(-0.1, 1.6, 0.4),
                ra: 0.5,
                rb: 0.15,
            },
        ];
        for prim in prims {
            for i in 0..4000 {
                let f = i as f32;
                let p = Vec3::new(
                    (f * 0.71).sin() * 3.0,
                    (f * 1.13).cos() * 3.0,
                    (f * 0.37).sin() * 3.0,
                );
                let d = prim.dist(p);
                assert!(d.is_finite(), "non-finite distance at {:?}", p);
                if d <= 0.0 {
                    continue;
                }
                // Step the reported distance toward a probe direction; the
                // surface must still be at least ~0 away, never crossed.
                let dir = Vec3::new((f * 2.1).cos(), (f * 0.9).sin(), (f * 1.7).cos()).normalize();
                let stepped = prim.dist(p + dir * d);
                assert!(
                    stepped > -1e-3,
                    "stepping the reported distance overshot the surface: \
                     d={} then {} at {:?}",
                    d,
                    stepped,
                    p
                );
            }
        }
    }

    /// Bounds must enclose the primitive or the BVH will prune away real hits.
    #[test]
    fn new_primitive_bounds_enclose_the_surface() {
        let prims = [
            Prim::Cylinder {
                a: Vec3::new(-0.4, 0.1, 0.2),
                b: Vec3::new(0.5, 1.3, -0.3),
                r: 0.35,
            },
            Prim::Cone {
                a: Vec3::new(0.2, 0.0, 0.1),
                b: Vec3::new(-0.1, 1.6, 0.4),
                ra: 0.5,
                rb: 0.15,
            },
        ];
        for prim in prims {
            let (lo, hi) = prim.bounds().expect("bounded primitive");
            for i in 0..20000 {
                let f = i as f32;
                let p = Vec3::new(
                    (f * 0.71).sin() * 2.5,
                    (f * 1.13).cos() * 2.5,
                    (f * 0.37).sin() * 2.5,
                );
                if prim.dist(p) < 0.0 {
                    assert!(
                        p.cmpge(lo - Vec3::splat(1e-4)).all()
                            && p.cmple(hi + Vec3::splat(1e-4)).all(),
                        "interior point {:?} lies outside bounds {:?}..{:?}",
                        p,
                        lo,
                        hi
                    );
                }
            }
        }
    }
}
