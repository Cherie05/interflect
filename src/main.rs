//! radiosity — a noise-free CPU renderer.
//!
//! Deterministic surfel radiosity on signed distance fields. No Monte Carlo,
//! no denoiser, no GPU, no model weights.

mod bvh;
mod film;
mod formfactor;
mod ltc;
mod reference;
mod scene;
mod sdf;
mod shade;
mod solve;
mod surfel;
mod trace;

use film::Film;
use glam::Vec3;
use rayon::prelude::*;
use scene::Scene;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Full render: analytic direct lighting plus solved indirect.
    Beauty,
    /// Direct lighting only. The difference against `beauty` is exactly what
    /// the radiosity solve contributes.
    Direct,
    /// Indirect only, so colour bleeding is visible in isolation.
    Indirect,
    /// Brute-force path-traced ground truth for the accuracy benchmark.
    Reference,
    Normals,
    Depth,
    Steps,
}

impl Mode {
    fn needs_gi(self) -> bool {
        matches!(self, Mode::Beauty | Mode::Indirect)
    }
    fn tonemapped(self) -> bool {
        matches!(
            self,
            Mode::Beauty | Mode::Direct | Mode::Indirect | Mode::Reference
        )
    }
}

struct Args {
    input: String,
    output: String,
    threads: usize,
    mode: Mode,
    no_bvh: bool,
    width: Option<u32>,
    height: Option<u32>,
    surfels: Option<usize>,
    bounces: Option<u32>,
    turntable: u32,
    spp: u32,
    cluster_res: i32,
}

fn usage() -> ! {
    eprintln!(
        "interflect — noise-free CPU renderer

USAGE:
    interflect render <scene.rad> [OPTIONS]

OPTIONS:
    -o, --output <FILE>    output PNG                  [default: out.png]
    -t, --threads <N>      worker threads              [default: all cores]
    -w, --width <N>        override scene width
    -h, --height <N>       override scene height
        --mode <MODE>      beauty | direct | indirect | reference | normals | depth | steps
        --spp <N>          samples/pixel for --mode reference   [default: 512]
        --clusters <N>     transfer cluster grid resolution     [default: 8]
        --surfels <N>      override surfel count
        --bounces <N>      override bounce count
        --turntable <N>    render N orbit frames reusing one GI solve
        --no-bvh           linear scan; for verifying BVH correctness
"
    );
    std::process::exit(2)
}

fn parse_args() -> Args {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.is_empty() || a[0] != "render" || a.len() < 2 {
        usage();
    }

    let mut args = Args {
        input: a[1].clone(),
        output: "out.png".into(),
        threads: 0,
        mode: Mode::Beauty,
        no_bvh: false,
        width: None,
        height: None,
        surfels: None,
        bounces: None,
        turntable: 0,
        spp: 512,
        cluster_res: 8,
    };

    let mut i = 2;
    while i < a.len() {
        let need = |i: usize| -> String {
            if i + 1 >= a.len() {
                usage();
            }
            a[i + 1].clone()
        };
        match a[i].as_str() {
            "-o" | "--output" => {
                args.output = need(i);
                i += 2;
            }
            "-t" | "--threads" => {
                args.threads = need(i).parse().unwrap_or(0);
                i += 2;
            }
            "-w" | "--width" => {
                args.width = need(i).parse().ok();
                i += 2;
            }
            "-h" | "--height" => {
                args.height = need(i).parse().ok();
                i += 2;
            }
            "--surfels" => {
                args.surfels = need(i).parse().ok();
                i += 2;
            }
            "--bounces" => {
                args.bounces = need(i).parse().ok();
                i += 2;
            }
            "--clusters" => {
                args.cluster_res = need(i).parse().unwrap_or(12);
                i += 2;
            }
            "--spp" => {
                args.spp = need(i).parse().unwrap_or(512);
                i += 2;
            }
            "--turntable" => {
                args.turntable = need(i).parse().unwrap_or(0);
                i += 2;
            }
            "--mode" => {
                args.mode = match need(i).as_str() {
                    "beauty" => Mode::Beauty,
                    "direct" => Mode::Direct,
                    "indirect" => Mode::Indirect,
                    "reference" => Mode::Reference,
                    "normals" => Mode::Normals,
                    "depth" => Mode::Depth,
                    "steps" => Mode::Steps,
                    other => {
                        eprintln!("unknown mode `{}`", other);
                        usage()
                    }
                };
                i += 2;
            }
            "--no-bvh" => {
                args.no_bvh = true;
                i += 1;
            }
            other => {
                eprintln!("unknown flag `{}`", other);
                usage()
            }
        }
    }
    args
}

/// Camera basis and per-pixel ray generation.
struct Cam {
    origin: Vec3,
    fwd: Vec3,
    right: Vec3,
    up: Vec3,
    tan_half: f32,
    aspect: f32,
}

impl Cam {
    fn new(pos: Vec3, look: Vec3, up_hint: Vec3, fov: f32, w: u32, h: u32) -> Cam {
        let fwd = (look - pos).normalize();
        let right = fwd.cross(up_hint).normalize();
        let up = right.cross(fwd);
        Cam {
            origin: pos,
            fwd,
            right,
            up,
            tan_half: (fov.to_radians() * 0.5).tan(),
            aspect: w as f32 / h as f32,
        }
    }

    #[inline]
    fn ray(&self, x: u32, y: u32, w: u32, h: u32) -> Vec3 {
        let px = (2.0 * (x as f32 + 0.5) / w as f32 - 1.0) * self.aspect * self.tan_half;
        let py = (1.0 - 2.0 * (y as f32 + 0.5) / h as f32) * self.tan_half;
        (self.fwd + self.right * px + self.up * py).normalize()
    }
}

/// Everything the shading pass needs from the GI solve. `None` for direct-only
/// modes.
struct Gi {
    surfels: Vec<surfel::Surfel>,
    grid: surfel::Grid,
    radius: f32,
}

#[allow(clippy::too_many_arguments)]
fn render_frame(
    sc: &Scene,
    cam: &Cam,
    mode: Mode,
    gi: Option<&Gi>,
    tmax: f32,
    spp: u32,
    film: &mut Film,
) -> u64 {
    let (w, h) = (film.width, film.height);
    let evals = AtomicU64::new(0);

    film.data
        .par_chunks_mut(w as usize)
        .enumerate()
        .for_each(|(y, row)| {
            let mut local = 0u64;
            for (x, px) in row.iter_mut().enumerate() {
                let rd = cam.ray(x as u32, y as u32, w, h);

                if mode == Mode::Reference {
                    // Seeded per pixel, so the reference is reproducible even
                    // though it is stochastic.
                    let mut rng = reference::Rng::new((y as u64) << 32 | x as u64).into_seeded();
                    let mut acc = Vec3::ZERO;
                    for _ in 0..spp {
                        acc += reference::path(sc, cam.origin, rd, tmax, 12, &mut rng);
                    }
                    *px = acc / spp as f32;
                    continue;
                }

                let (hit, steps) = trace::trace(sc, cam.origin, rd, 1e-3, tmax);
                local += steps as u64;

                let surf_t = hit.map_or(f32::MAX, |hh| hh.t);
                if let Some((_, e)) =
                    shade::emitter_hit(&sc.lights, cam.origin, rd, surf_t.min(tmax))
                {
                    *px = if mode.tonemapped() {
                        e
                    } else {
                        Vec3::splat(0.9)
                    };
                    continue;
                }

                *px = match hit {
                    None => Vec3::new(0.02, 0.03, 0.05),
                    Some(hh) => match mode {
                        Mode::Normals => hh.n * 0.5 + Vec3::splat(0.5),
                        Mode::Depth => {
                            let d = 1.0 - (hh.t / tmax).clamp(0.0, 1.0);
                            Vec3::splat(d * d)
                        }
                        Mode::Steps => {
                            let f = steps as f32 / trace::MAX_STEPS as f32;
                            Vec3::new(f, 1.0 - f, 0.2)
                        }
                        Mode::Reference => Vec3::ZERO, // handled above
                        Mode::Direct | Mode::Beauty | Mode::Indirect => {
                            let m = sc.material(hh.mat);
                            let mut c = Vec3::ZERO;

                            if mode != Mode::Indirect {
                                let (d, s) = shade::direct(sc, hh.p, hh.n, -rd, &m);
                                local += s as u64;
                                c += d + m.emissive;
                            }

                            if let Some(g) = gi {
                                let irr = surfel::gather(&g.surfels, &g.grid, hh.p, hh.n, g.radius);
                                // No ambient-occlusion term here, deliberately.
                                // AO is a stand-in for occlusion that an engine
                                // cannot compute properly — but the transfer
                                // matrix already carries cone-traced visibility
                                // per link, so multiplying by AO double-counts
                                // it. Worse, AO tends to zero in a concave
                                // corner while true global illumination gets
                                // *brighter* there (two surfaces bouncing into
                                // each other), so the combination drew black
                                // seams along every wall junction.
                                c += shade::diffuse_albedo(&m) * irr;

                                // --- specular indirect ---
                                // Without this a metal has no diffuse lobe and
                                // no reflection, so it renders black no matter
                                // how bright the room is. One reflection ray,
                                // shaded with the cached irradiance, is enough
                                // to make mirrors and glossy surfaces read
                                // correctly — and it stays noise-free because
                                // the ray direction is the exact mirror
                                // direction, not a sampled lobe.
                                if m.roughness < 0.45 || m.metallic > 0.05 {
                                    let refl = rd - hh.n * (2.0 * hh.n.dot(rd));
                                    let (rh, s) =
                                        trace::trace(sc, hh.p + hh.n * 1e-3, refl, 1e-3, tmax);
                                    local += s as u64;

                                    let rt = rh.map_or(f32::MAX, |x| x.t);
                                    let refl_col = if let Some((_, e)) =
                                        shade::emitter_hit(&sc.lights, hh.p, refl, rt.min(tmax))
                                    {
                                        e
                                    } else if let Some(r2) = rh {
                                        let m2 = sc.material(r2.mat);
                                        let (d2, s2) = shade::direct(sc, r2.p, r2.n, -refl, &m2);
                                        local += s2 as u64;
                                        let i2 = surfel::gather(
                                            &g.surfels, &g.grid, r2.p, r2.n, g.radius,
                                        );
                                        d2 + shade::diffuse_albedo(&m2) * i2 + m2.emissive
                                    } else {
                                        Vec3::new(0.02, 0.03, 0.05)
                                    };

                                    // Fresnel gate, and roughness attenuates the
                                    // mirror term rather than blurring it: a
                                    // blurred reflection would need a cone
                                    // trace, which is the honest next upgrade.
                                    let f = shade::fresnel_schlick(
                                        hh.n.dot(-rd).max(0.0),
                                        shade::f0_of(&m),
                                    );
                                    let gloss = (1.0 - m.roughness / 0.45).clamp(0.0, 1.0);
                                    let gloss = gloss * gloss;
                                    c += f * refl_col * gloss;
                                }
                            }
                            c
                        }
                    },
                };
            }
            evals.fetch_add(local, Ordering::Relaxed);
        });

    evals.load(Ordering::Relaxed)
}

fn main() {
    let args = parse_args();

    if args.threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(args.threads)
            .build_global()
            .ok();
    }

    let src = match std::fs::read_to_string(&args.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {}", args.input, e);
            std::process::exit(1);
        }
    };

    let mut sc = match scene::parse(&src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}: {}", args.input, e);
            std::process::exit(1);
        }
    };
    sc.use_bvh = !args.no_bvh;
    if let Some(w) = args.width {
        sc.cfg.width = w;
    }
    if let Some(h) = args.height {
        sc.cfg.height = h;
    }
    if let Some(n) = args.surfels {
        sc.cfg.surfels = n;
    }
    if let Some(b) = args.bounces {
        sc.cfg.bounces = b;
    }

    // CLI flags bypass the parser, so the same limits have to be re-checked
    // here or `--surfels 999999999` walks straight past them.
    if let Err(e) = sc.cfg.validate() {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }

    let (w, h) = (sc.cfg.width, sc.cfg.height);
    let diag = (sc.bounds.1 - sc.bounds.0).length();
    let tmax = diag * 4.0;

    println!("interflect {}", env!("CARGO_PKG_VERSION"));
    println!(
        "  scene      {} objects, {} lights, {} materials, {} bvh nodes",
        sc.objects.len(),
        sc.lights.len(),
        sc.materials.len() - 1,
        sc.bvh.node_count()
    );
    println!("  film       {}x{}", w, h);

    // ---------------------------------------------------------------------
    // Global illumination solve (view-independent; done once)
    // ---------------------------------------------------------------------
    let t_gi = Instant::now();
    let gi = if args.mode.needs_gi() {
        let t = Instant::now();
        let mut surfels = surfel::generate(&sc, sc.cfg.surfels);
        let gen_ms = t.elapsed().as_secs_f64() * 1e3;
        if surfels.is_empty() {
            eprintln!("error: no surfels generated — scene may have no surface inside its bounds");
            std::process::exit(1);
        }
        println!(
            "  surfels    {} placed of {} requested   ({:.0} ms)",
            surfels.len(),
            sc.cfg.surfels,
            gen_ms
        );

        let t = Instant::now();
        let clusters = formfactor::build_clusters(&sc, &surfels, args.cluster_res);
        let (transfer, ff_evals) = formfactor::build(&sc, &surfels, clusters);
        let ff_ms = t.elapsed().as_secs_f64() * 1e3;
        println!(
            "  transfer   {} clusters, {} links, row-sum {:.3}, {:.1}M sdf evals   ({:.0} ms)",
            transfer.nc,
            transfer.links,
            transfer.mean_row_sum,
            ff_evals as f64 / 1e6,
            ff_ms
        );

        let t = Instant::now();
        let direct_evals = solve::light_surfels(&sc, &mut surfels);
        let stats = solve::solve(&mut surfels, &transfer, sc.cfg.bounces);
        let solve_ms = t.elapsed().as_secs_f64() * 1e3;
        println!(
            "  solve      {} bounces, residual {:.2e}, {:.1}M sdf evals   ({:.0} ms)",
            stats.iterations,
            stats.residual,
            direct_evals as f64 / 1e6,
            solve_ms
        );

        let radius = (diag / (sc.cfg.surfels as f32).sqrt()) * 2.6;
        let pts: Vec<Vec3> = surfels.iter().map(|s| s.p).collect();
        let grid = surfel::Grid::build(&pts, sc.bounds.0, sc.bounds.1, radius);
        Some(Gi {
            surfels,
            grid,
            radius,
        })
    } else {
        None
    };
    let gi_ms = t_gi.elapsed().as_secs_f64() * 1e3;

    // ---------------------------------------------------------------------
    // Shading passes
    // ---------------------------------------------------------------------
    let mut film = Film::new(w, h);
    let c = sc.camera;

    if args.turntable > 0 {
        // The whole point of a view-independent solve: the lighting above is
        // reused verbatim, and each extra camera costs only the gather pass.
        let base = c.pos - c.look;
        let n = args.turntable;
        let t0 = Instant::now();
        for i in 0..n {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            let (s, co) = a.sin_cos();
            let pos =
                c.look + Vec3::new(base.x * co - base.z * s, base.y, base.x * s + base.z * co);
            let cam = Cam::new(pos, c.look, c.up, c.fov, w, h);
            render_frame(&sc, &cam, args.mode, gi.as_ref(), tmax, args.spp, &mut film);
            let path = format!("{}_{:03}.png", args.output.trim_end_matches(".png"), i);
            if let Err(e) = film.save(&path, sc.cfg.exposure, args.mode.tonemapped()) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        }
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        println!(
            "  turntable  {} frames in {:.0} ms  ({:.0} ms/frame, GI solved once in {:.0} ms)",
            n,
            ms,
            ms / n as f64,
            gi_ms
        );
        return;
    }

    let cam = Cam::new(c.pos, c.look, c.up, c.fov, w, h);
    let t0 = Instant::now();
    let evals = render_frame(&sc, &cam, args.mode, gi.as_ref(), tmax, args.spp, &mut film);
    let ms = t0.elapsed().as_secs_f64() * 1e3;

    let px = (w as u64) * (h as u64);
    println!(
        "  shade      {:.0} ms   {:.1}M sdf evals   {:.0} evals/px",
        ms,
        evals as f64 / 1e6,
        evals as f64 / px as f64
    );
    println!("  total      {:.0} ms", gi_ms + ms);

    if let Err(e) = film.save(&args.output, sc.cfg.exposure, args.mode.tonemapped()) {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
    println!("  wrote      {}", args.output);
}
