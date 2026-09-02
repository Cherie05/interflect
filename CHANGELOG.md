# Changelog

Notable changes to Interflect. Format follows [Keep a Changelog](https://keepachangelog.com/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] — 2026-09-02

First release.

### The idea

Classical radiosity produced noise-free, view-independent global illumination and
then lost to path tracing, because it needed every surface subdivided into
well-conditioned patches and automatic meshing was fragile. A signed distance
field has no mesh to subdivide — any point projects onto the surface by Newton
iteration along the gradient — so the obstacle that sidelined the method is not
there. With no GPU and no denoiser permitted, it wins.

### Added

- **Renderer** — meshless surfel placement on an SDF feeding a classical
  radiosity solve. Sphere-traced geometry, binned-SAH BVH over nearest-distance
  queries, analytic polygonal area lights, cone-traced soft shadows, sparse CSR
  transfer matrix, Jacobi solve.
- **Primitives** — sphere, rounded box, plane, capsule, cylinder, capped cone,
  torus, with CSG subtraction.
- **View-independent solve** — `--turntable N` renders N orbit frames from a
  single GI solve.
- **Modes** — `beauty`, `direct`, `indirect`, `reference` (a built-in
  brute-force path tracer used as benchmark ground truth), plus `normals`,
  `depth` and `steps` for debugging.
- **`.rad` scene format** — plain text; the renderer never needs recompiling.
- **Scene builder** (`tools/scene-builder.html`) — offline, single-file, with
  drag-editable front and top views and live scene output.
- **Benchmark harness** (`bench.sh`) — renders each scene against the built-in
  path tracer and reports SSIM, speedup and energy ratio.
- 25 regression tests. Every test in `bvh`, `ltc`, `trace`, `formfactor` and
  `solve` reproduces the failure signature of a real bug found during
  development.
- CI gating determinism, energy conservation, solve convergence, `--no-bvh`
  correctness, accuracy against ground truth, and malformed-input handling on
  Linux, Windows and macOS.

### Measured

On a 6-core / 12-thread AMD Ryzen 5 4600H, 260×260, against the built-in path
tracer at 384 spp:

| Scene | speedup | SSIM | energy ratio |
|---|---|---|---|
| `sphere_only` | 18.9× | 0.850 | 1.090 |
| `box_only` | 22.3× | 0.839 | 1.066 |
| `high_albedo` | 22.4× | 0.820 | 1.049 |
| `cornell` | 17.6× | 0.806 | 1.050 |

Read SSIM against a measured ceiling of 0.892, not 1.0: the reference is
stochastic and SSIM penalises its own noise even against a perfect image.

Output is bit-identical across thread counts and across runs.

### Known limitations

No caustics, no refraction, blurry reflections attenuated rather than blurred,
diffuse-dominant GI, no participating media, analytic primitives only. Faint
contour banding remains in penumbrae where a shadow ray runs nearly parallel to
a large surface.

[Unreleased]: https://github.com/Cherie05/interflect/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Cherie05/interflect/releases/tag/v0.1.0
