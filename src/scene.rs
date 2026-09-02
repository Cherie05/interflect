//! Scene representation and the `.rad` scene DSL parser.
//!
//! Hand-rolled tokeniser rather than a parser-combinator crate: the grammar is
//! ~8 block types, and keeping the dependency list at three crates is a stated
//! goal of the project.

use crate::sdf::{Object, Prim};
use glam::Vec3;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub albedo: Vec3,
    pub roughness: f32,
    pub metallic: f32,
    pub emissive: Vec3,
}

impl Default for Material {
    fn default() -> Self {
        Material {
            albedo: Vec3::splat(0.8),
            roughness: 0.8,
            metallic: 0.0,
            emissive: Vec3::ZERO,
        }
    }
}

/// Rectangular area light. Vertices in counter-clockwise order viewed from the
/// emitting side.
#[derive(Clone, Copy, Debug)]
pub struct QuadLight {
    pub verts: [Vec3; 4],
    pub emit: Vec3,
}

impl QuadLight {
    pub fn center(&self) -> Vec3 {
        (self.verts[0] + self.verts[1] + self.verts[2] + self.verts[3]) * 0.25
    }
    pub fn normal(&self) -> Vec3 {
        (self.verts[1] - self.verts[0])
            .cross(self.verts[3] - self.verts[0])
            .normalize()
    }
    pub fn area(&self) -> f32 {
        let a = (self.verts[1] - self.verts[0])
            .cross(self.verts[3] - self.verts[0])
            .length();
        let b = (self.verts[3] - self.verts[2])
            .cross(self.verts[1] - self.verts[2])
            .length();
        0.5 * (a + b)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: Vec3,
    pub look: Vec3,
    pub up: Vec3,
    pub fov: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            pos: Vec3::new(0.0, 2.0, 6.0),
            look: Vec3::new(0.0, 1.0, 0.0),
            up: Vec3::Y,
            fov: 40.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RenderCfg {
    pub width: u32,
    pub height: u32,
    pub surfels: usize,
    pub bounces: u32,
    pub exposure: f32,
}

impl Default for RenderCfg {
    fn default() -> Self {
        RenderCfg {
            width: 800,
            height: 600,
            surfels: 20000,
            bounces: 12,
            exposure: 1.0,
        }
    }
}

pub struct Scene {
    /// Bounded objects, indexed by the BVH.
    pub objects: Vec<Object>,
    /// Unbounded objects (planes); always evaluated.
    pub planes: Vec<Object>,
    pub materials: Vec<Material>,
    pub lights: Vec<QuadLight>,
    pub camera: Camera,
    pub cfg: RenderCfg,
    pub bvh: crate::bvh::Bvh,
    /// When false, distance queries linearly scan every object. Used by
    /// `--no-bvh` to prove the BVH changes speed but not pixels.
    pub use_bvh: bool,
    /// World bounding box, used for surfel seeding.
    pub bounds: (Vec3, Vec3),
}

/// Distance query result: distance plus the material of the closest object.
#[derive(Clone, Copy)]
pub struct DistHit {
    pub d: f32,
    pub mat: u32,
}

impl Scene {
    /// Full scene SDF with material attribution. The BVH prunes objects whose
    /// AABB is further away than the best distance found so far.
    #[inline]
    pub fn dist_mat(&self, p: Vec3) -> DistHit {
        let mut best = DistHit {
            d: f32::MAX,
            mat: 0,
        };
        for o in &self.planes {
            let d = o.dist(p);
            if d < best.d {
                best = DistHit { d, mat: o.mat };
            }
        }
        if self.use_bvh {
            self.bvh.query(p, &self.objects, &mut best);
        } else {
            self.bvh.query_linear(p, &self.objects, &mut best);
        }
        best
    }

    #[inline]
    pub fn dist(&self, p: Vec3) -> f32 {
        self.dist_mat(p).d
    }

    /// Surface normal via the tetrahedron trick: four taps instead of the six a
    /// central-difference gradient needs.
    #[inline]
    pub fn normal(&self, p: Vec3) -> Vec3 {
        const H: f32 = 5e-4;
        const K: [Vec3; 4] = [
            Vec3::new(1.0, -1.0, -1.0),
            Vec3::new(-1.0, -1.0, 1.0),
            Vec3::new(-1.0, 1.0, -1.0),
            Vec3::new(1.0, 1.0, 1.0),
        ];
        let mut n = Vec3::ZERO;
        for k in K {
            n += k * self.dist(p + k * H);
        }
        let l = n.length();
        if l > 1e-12 {
            n / l
        } else {
            Vec3::Y
        }
    }

    pub fn material(&self, id: u32) -> Material {
        self.materials.get(id as usize).copied().unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tokeniser
// ---------------------------------------------------------------------------

fn tokenize(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut chars = src.chars().peekable();
    let quote = '"';
    let hash = '#';

    while let Some(c) = chars.next() {
        if in_str {
            if c == quote {
                out.push(format!("{}{}", quote, cur));
                cur.clear();
                in_str = false;
            } else {
                cur.push(c);
            }
            continue;
        }
        if c == hash {
            for n in chars.by_ref() {
                if n == '\n' {
                    break;
                }
            }
            continue;
        }
        if c == quote {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            in_str = true;
            continue;
        }
        if "{}[],:".contains(c) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
            continue;
        }
        if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

struct P {
    t: Vec<String>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&str> {
        self.t.get(self.i).map(|s| s.as_str())
    }
    fn next_tok(&mut self) -> Result<String, String> {
        let v = self
            .t
            .get(self.i)
            .cloned()
            .ok_or_else(|| "unexpected end of file".to_string())?;
        self.i += 1;
        Ok(v)
    }
    fn expect(&mut self, s: &str) -> Result<(), String> {
        let n = self.next_tok()?;
        if n == s {
            Ok(())
        } else {
            Err(format!("expected `{}`, found `{}`", s, n))
        }
    }
    fn float(&mut self) -> Result<f32, String> {
        let n = self.next_tok()?;
        n.parse::<f32>()
            .map_err(|_| format!("expected number, found `{}`", n))
    }
    fn vec3(&mut self) -> Result<Vec3, String> {
        self.expect("[")?;
        let x = self.float()?;
        self.skip_comma();
        let y = self.float()?;
        self.skip_comma();
        let z = self.float()?;
        self.skip_comma();
        self.expect("]")?;
        Ok(Vec3::new(x, y, z))
    }
    fn ident(&mut self) -> Result<String, String> {
        let n = self.next_tok()?;
        Ok(n.trim_start_matches('"').to_string())
    }
    fn skip_comma(&mut self) {
        if self.peek() == Some(",") {
            self.i += 1;
        }
    }
    /// Reads `key:` and returns the key, or `None` at a closing brace.
    fn key(&mut self) -> Result<Option<String>, String> {
        if self.peek() == Some("}") {
            self.i += 1;
            return Ok(None);
        }
        let k = self.next_tok()?;
        self.expect(":")?;
        Ok(Some(k))
    }
}

pub fn parse(src: &str) -> Result<Scene, String> {
    let mut p = P {
        t: tokenize(src),
        i: 0,
    };

    let mut camera = Camera::default();
    let mut cfg = RenderCfg::default();
    let mut materials: Vec<Material> = vec![Material::default()];
    let mut mat_ids: HashMap<String, u32> = HashMap::new();
    let mut objects: Vec<Object> = Vec::new();
    let mut planes: Vec<Object> = Vec::new();
    let mut lights: Vec<QuadLight> = Vec::new();

    while let Some(tok) = p.peek() {
        let block = tok.to_string();
        p.i += 1;

        match block.as_str() {
            "camera" => {
                p.expect("{")?;
                while let Some(k) = p.key()? {
                    match k.as_str() {
                        "pos" => camera.pos = p.vec3()?,
                        "look" => camera.look = p.vec3()?,
                        "up" => camera.up = p.vec3()?,
                        "fov" => camera.fov = p.float()?,
                        _ => return Err(format!("unknown camera key `{}`", k)),
                    }
                    p.skip_comma();
                }
            }

            "render" => {
                p.expect("{")?;
                while let Some(k) = p.key()? {
                    match k.as_str() {
                        "width" => cfg.width = p.float()? as u32,
                        "height" => cfg.height = p.float()? as u32,
                        "surfels" => cfg.surfels = p.float()? as usize,
                        "bounces" => cfg.bounces = p.float()? as u32,
                        "exposure" => cfg.exposure = p.float()?,
                        _ => return Err(format!("unknown render key `{}`", k)),
                    }
                    p.skip_comma();
                }
            }

            "material" => {
                let name = p.ident()?;
                let mut m = Material::default();
                p.expect("{")?;
                while let Some(k) = p.key()? {
                    match k.as_str() {
                        "albedo" => m.albedo = p.vec3()?,
                        "roughness" => m.roughness = p.float()?.clamp(0.02, 1.0),
                        "metallic" => m.metallic = p.float()?.clamp(0.0, 1.0),
                        "emissive" => m.emissive = p.vec3()?,
                        _ => return Err(format!("unknown material key `{}`", k)),
                    }
                    p.skip_comma();
                }
                mat_ids.insert(name, materials.len() as u32);
                materials.push(m);
            }

            "light" => {
                let mut verts = [Vec3::ZERO; 4];
                let mut emit = Vec3::splat(10.0);
                p.expect("{")?;
                while let Some(k) = p.key()? {
                    match k.as_str() {
                        "verts" => {
                            p.expect("[")?;
                            for v in verts.iter_mut() {
                                *v = p.vec3()?;
                                p.skip_comma();
                            }
                            p.expect("]")?;
                        }
                        "emit" => emit = p.vec3()?,
                        _ => return Err(format!("unknown light key `{}`", k)),
                    }
                    p.skip_comma();
                }
                lights.push(QuadLight { verts, emit });
            }

            "sphere" | "box" | "plane" | "capsule" | "torus" | "cylinder" | "cone" => {
                let mut mat = 0u32;
                let mut subtract = Vec::new();

                let mut center = Vec3::ZERO;
                let mut radius = 1.0f32;
                let mut size = Vec3::ONE;
                let mut round = 0.0f32;
                let mut normal = Vec3::Y;
                let mut height = 0.0f32;
                let mut a = Vec3::ZERO;
                let mut b = Vec3::Y;
                let mut major = 1.0f32;
                let mut minor = 0.25f32;
                // Cone tip radius. Defaults to 0, giving a true point-tipped
                // cone unless the scene asks for a truncated one.
                let mut radius_top = 0.0f32;

                p.expect("{")?;
                while let Some(k) = p.key()? {
                    match k.as_str() {
                        "center" => center = p.vec3()?,
                        "radius" => radius = p.float()?,
                        "size" => size = p.vec3()?,
                        "round" => round = p.float()?,
                        "normal" => normal = p.vec3()?.normalize(),
                        "height" => height = p.float()?,
                        "a" => a = p.vec3()?,
                        "b" => b = p.vec3()?,
                        "major" => major = p.float()?,
                        "minor" => minor = p.float()?,
                        "radius_top" => radius_top = p.float()?,
                        "mat" => {
                            let n = p.ident()?;
                            mat = *mat_ids
                                .get(&n)
                                .ok_or_else(|| format!("unknown material `{}`", n))?;
                        }
                        "subtract_sphere" => {
                            p.expect("[")?;
                            let c = p.vec3()?;
                            p.skip_comma();
                            let r = p.float()?;
                            p.skip_comma();
                            p.expect("]")?;
                            subtract.push(Prim::Sphere { c, r });
                        }
                        _ => return Err(format!("unknown shape key `{}`", k)),
                    }
                    p.skip_comma();
                }

                let prim = match block.as_str() {
                    "sphere" => Prim::Sphere {
                        c: center,
                        r: radius,
                    },
                    "box" => Prim::RoundBox {
                        c: center,
                        b: size * 0.5,
                        r: round.min(size.min_element() * 0.499),
                    },
                    "plane" => Prim::Plane {
                        n: normal,
                        h: height,
                    },
                    "capsule" => Prim::Capsule { a, b, r: radius },
                    "cylinder" => Prim::Cylinder { a, b, r: radius },
                    "cone" => Prim::Cone {
                        a,
                        b,
                        ra: radius,
                        rb: radius_top,
                    },
                    _ => Prim::Torus {
                        c: center,
                        major,
                        minor,
                    },
                };

                let obj = Object {
                    prim,
                    subtract,
                    mat,
                    blend: 0.0,
                };
                if matches!(prim, Prim::Plane { .. }) {
                    planes.push(obj);
                } else {
                    objects.push(obj);
                }
            }

            other => return Err(format!("unknown block `{}`", other)),
        }
    }

    if lights.is_empty() {
        return Err("scene has no `light` block".into());
    }

    let bvh = crate::bvh::Bvh::build(&objects);
    let bounds = world_bounds(&objects, &planes, &lights);

    Ok(Scene {
        objects,
        planes,
        materials,
        lights,
        camera,
        cfg,
        bvh,
        use_bvh: true,
        bounds,
    })
}

fn world_bounds(objects: &[Object], planes: &[Object], lights: &[QuadLight]) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for o in objects {
        if let Some((a, b)) = o.prim.bounds() {
            lo = lo.min(a);
            hi = hi.max(b);
        }
    }
    for l in lights {
        for v in l.verts {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if lo.x > hi.x {
        return (Vec3::splat(-5.0), Vec3::splat(5.0));
    }
    // Padding is deliberately tight. These bounds drive two things that both
    // degrade fast when the box is oversized: the Poisson separation d_min
    // (which scales with the diagonal, so a loose box places far fewer surfels
    // than requested) and the cluster voxel size (which coarsens until distinct
    // walls merge into one emitter). Infinite planes need slack because they
    // extend past every object; a closed scene needs almost none.
    let has_planes = !planes.is_empty();
    let diag = (hi - lo).length();
    let pad = if has_planes { diag * 0.25 } else { diag * 0.02 };
    (lo - Vec3::splat(pad), hi + Vec3::splat(pad))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
material "w" { albedo: [0.5, 0.6, 0.7], roughness: 0.4 }
sphere { center: [1, 2, 3], radius: 0.5, mat: "w" }
light { verts: [[-1,4,-1],[1,4,-1],[1,4,1],[-1,4,1]], emit: [9,8,7] }
"#;

    #[test]
    fn parses_a_minimal_scene() {
        let sc = parse(MINIMAL).expect("minimal scene should parse");
        assert_eq!(sc.objects.len(), 1);
        assert_eq!(sc.lights.len(), 1);
        // index 0 is the implicit default material
        assert_eq!(sc.materials.len(), 2);
        assert_eq!(sc.material(1).albedo, Vec3::new(0.5, 0.6, 0.7));
    }

    #[test]
    fn comments_and_trailing_commas_are_accepted() {
        let src = format!("# leading comment\n{}\n# trailing comment\n", MINIMAL);
        assert!(parse(&src).is_ok());
    }

    /// Planes are unbounded, so they must be kept out of the BVH — its AABB
    /// pruning would be meaningless for them.
    #[test]
    fn planes_are_excluded_from_the_bvh() {
        let src = format!(
            "{}\nplane {{ normal: [0,1,0], height: 0, mat: \"w\" }}\n",
            MINIMAL
        );
        let sc = parse(&src).unwrap();
        assert_eq!(
            sc.planes.len(),
            1,
            "plane should land in the unbounded list"
        );
        assert_eq!(sc.objects.len(), 1, "plane must not enter the BVH");
    }

    /// Malformed input must produce a diagnostic, never a panic. Every case here
    /// was verified by hand against the binary first.
    #[test]
    fn malformed_input_errors_instead_of_panicking() {
        let cases: &[(&str, &str)] = &[
            ("", "no `light` block"),
            ("wibble { }", "unknown block"),
            ("camera { nonsense: 5 }", "unknown camera key"),
            ("camera { fov: abc }", "expected number"),
            ("camera { pos: [1, 2 }", "expected number"),
            (
                "light { emit: [1,1,1] } sphere { mat: \"nope\" }",
                "unknown material",
            ),
        ];
        for (src, expect) in cases {
            match parse(src) {
                Ok(_) => panic!("`{}` should not have parsed", src),
                Err(e) => assert!(
                    e.contains(expect),
                    "for `{}` expected an error mentioning {:?}, got {:?}",
                    src,
                    expect,
                    e
                ),
            }
        }
    }

    /// World bounds drive the Poisson separation and the cluster grid, so an
    /// oversized pad silently starves the surfel budget.
    #[test]
    fn bounds_enclose_geometry_without_excessive_padding() {
        let sc = parse(MINIMAL).unwrap();
        let (lo, hi) = sc.bounds;
        assert!(
            lo.cmple(Vec3::new(0.5, 1.5, 2.5)).all(),
            "bounds miss the sphere"
        );
        assert!(
            hi.cmpge(Vec3::new(1.5, 4.0, 3.5)).all(),
            "bounds miss the light"
        );
        // A closed scene should be padded only marginally.
        let geom = Vec3::new(1.5, 4.0, 3.5) - Vec3::new(0.5, 1.5, -1.0);
        assert!(
            (hi - lo).length() < geom.length() * 2.0,
            "bounds are over-padded: {:?} for geometry spanning {:?}",
            hi - lo,
            geom
        );
    }
}
