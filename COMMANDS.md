# Commands

Every command below was run on the machine that produced the numbers in
[README.md](README.md) — Windows 11, Rust 1.97.1, AMD Ryzen 5 4600H (6 cores / 12 threads). They work unchanged
on Linux and macOS. Shell is Git Bash / POSIX `sh`; on PowerShell use
`.\target\release\radiosity.exe` and swap `md5sum` for `Get-FileHash`.

---

## 1. Build

```bash
cd radiosity
cargo build --release
```

Produces two binaries in `target/release/`:

| Binary | Purpose |
|---|---|
| `radiosity` | the renderer |
| `compare` | image diff: SSIM, max/mean error, luminance ratio, diff map |

Expect a clean build — **zero warnings**. Any warning is a regression.

---

## 2. Run the regression suite

```bash
cargo test --release
```

Expected: **21 passed; 0 failed**. For a fuller testing guide — tiers, coverage
gaps, and a visual defect catalogue — see [TESTING.md](TESTING.md). Use `--release`; the debug build runs the
surfel and transfer stages 20–50× slower.

Each test reproduces the failure signature of a specific bug found during
development. If one fails, the message names the defect rather than just the
assertion.

| Test | Guards against |
|---|---|
| `bvh::child_indices_are_consistent_with_layout` | Child-allocation order. Traversal assumes the left child sits at `parent + 1`, which only holds if the left subtree is fully built before the right child is pushed. A mis-indexed child's AABB escapes its parent — that containment check is what detects it. |
| `bvh::every_primitive_lands_in_exactly_one_leaf` | Partition errors dropping or duplicating geometry. |
| `bvh::pruning_agrees_with_linear_scan` | AABB distance is a strict lower bound, so pruning must never discard a closer surface. 3000 sample points. |
| `ltc::dsl_winding_yields_positive_form_factor` | Light winding. The form factor sums *signed* edge contributions and needs the polygon CCW as seen from the **receiver**; the DSL authors it from the **emitter**. Without the flip every form factor goes negative, clamps to zero, and the scene renders black with no other symptom. |
| `ltc::small_distant_light_matches_analytic_limit` | Quantitative check against `A / (π d²)`. Catches sign and scale errors a positivity test alone would miss. |
| `ltc::light_below_horizon_contributes_nothing` | Horizon clipping; a light below the surface must contribute 0, not a negative value. |
| `trace::unoccluded_floor_under_flush_light_is_not_self_shadowed` | Shadow-cone self-occlusion. `h` is an *omnidirectional* distance, so a ray climbing toward a light flush-mounted in a ceiling runs parallel to that ceiling, reports a tiny `h`, and darkens a floor in plain view of the light. |
| `trace::real_occluder_still_casts_shadow` | The complement — proves the fix above is not "always return 1". |
| `formfactor::closed_box_conserves_energy` | Energy conservation. Raw point-to-disc form factors summed to **0.213** inside a sealed box whose true value is exactly 1.0, leaving the scene ~5× too dark. Asserts the interior mean now exceeds 0.7 and never exceeds 1.0. |
| `formfactor::transfer_matrix_is_sparse_and_well_formed` | The dense matrix was **208 MB** and mostly structural zeros. Validates CSR structure and that the matrix is genuinely sparse. |
| `solve::no_interior_surfel_is_unlit` | AO double-counting. The transfer matrix already carries per-link visibility; multiplying by AO drew black seams along every wall junction, where real GI gets *brighter*. |
| `solve::solve_converges` | Energy must strictly decrease per bounce for albedo < 1. |

### Run one test with output

```bash
cargo test --release closed_box_conserves_energy -- --nocapture
```

---

## 3. Render

```bash
# defaults come from the scene file
./target/release/radiosity render scenes/product.rad -o out.png

# override resolution and quality
./target/release/radiosity render scenes/cornell.rad -o out.png \
    -w 1920 -h 1080 --surfels 16000 --bounces 16
```

### Seeing what the radiosity solve contributes

```bash
./target/release/radiosity render scenes/cornell.rad -o direct.png   --mode direct
./target/release/radiosity render scenes/cornell.rad -o beauty.png   --mode beauty
./target/release/radiosity render scenes/cornell.rad -o indirect.png --mode indirect
./target/release/compare direct.png beauty.png --diff gi_only.png
```

`direct.png` has fully black shadows and an unlit block face. Everything the eye
reads as fill light in `beauty.png` comes from the solve.

### Debug modes

```bash
./target/release/radiosity render scenes/cornell.rad -o n.png --mode normals
./target/release/radiosity render scenes/cornell.rad -o d.png --mode depth
./target/release/radiosity render scenes/cornell.rad -o s.png --mode steps   # green=cheap, red=expensive
```

### Turntable — the view-independence payoff

```bash
./target/release/radiosity render scenes/product.rad -o tt.png --turntable 12 -w 480 -h 360
```

Writes `tt_000.png` … `tt_011.png`. The GI is solved **once**; each extra camera
pays only the gather pass. Measured: 429 ms solve, then 80 ms/frame.

Use an open scene. Orbiting a sealed room like `cornell.rad` puts the camera
outside it, and most frames render black.

---

## 4. Benchmark

```bash
./bench.sh                      # defaults: 384 spp, 260px
SPP=1024 RES=400 ./bench.sh     # slower, tighter reference
```

Renders every scene in `scenes/bench/` twice — once with the solver, once with
the built-in path tracer as ground truth — then reports SSIM, speedup and
energy ratio, and finishes with the determinism gate.

**Reading the SSIM column:** the reference is stochastic and SSIM penalises its
residual noise even against a perfect image. Two references of the *same* scene
differing only in sample count score **0.892** against each other, so ~0.89 is
the ceiling, not 1.0. Establish it yourself:

```bash
./target/release/radiosity render scenes/bench/cornell.rad -o r128.png --mode reference --spp 128
./target/release/radiosity render scenes/bench/cornell.rad -o r512.png --mode reference --spp 512
./target/release/compare r128.png r512.png
```

Scenes in `scenes/bench/` are all Lambertian by necessity — the reference
integrator is diffuse-only, so a glossy scene would measure the gap between two
BSDFs rather than the accuracy of the light transport.

---

## 5. Verification gates

These are integration-level; they cannot be expressed as unit tests.

### Determinism — bit-identical across thread counts

```bash
for t in 1 3 8 12; do
  ./target/release/radiosity render scenes/cornell.rad -o det_$t.png -w 300 -h 300 -t $t
done
md5sum det_*.png
```

**Pass:** all four hashes identical. Anything else means a float reduction is
summing in nondeterministic order.

### Determinism — reproducible across runs

```bash
for i in 1 2 3; do
  ./target/release/radiosity render scenes/cornell.rad -o rep_$i.png -w 300 -h 300
done
md5sum rep_*.png
```

### The `--no-bvh` flag is actually wired

This gate exists because the flag was once parsed and never applied, so the BVH
correctness check passed **vacuously** with identical timings. Timing is the only
observable difference, since both paths must produce the same pixels.

```bash
echo "with BVH:";    ./target/release/radiosity render scenes/stress.rad -o b.png --mode normals | grep shade
echo "linear scan:"; ./target/release/radiosity render scenes/stress.rad -o l.png --mode normals --no-bvh | grep shade
./target/release/compare b.png l.png
```

**Pass:** linear is >2× slower (measured **7.8×** on 513 primitives) **and** SSIM
is 1.000000.

A handful of pixels may differ at silhouette edges (measured: 2 in 360k, max
7/255). That is not a pruning error — LLVM makes different auto-vectorisation and
FMA-contraction choices for the two loops, so their last bits differ, and a
sphere-tracing ray sitting exactly on the hit epsilon can fall either way.

### Energy conservation

```bash
./target/release/radiosity render scenes/cornell.rad -o /tmp/scratch.png --surfels 16000 | grep transfer
```

**Pass:** `row-sum` ≳ 0.8. It was 0.213 before the hemisphere-closure
calibration. The figure averages interior surfels only — surfels also land on
the *outside* of wall slabs and correctly see nothing; including them halves the
number and hides the one that matters.

### Solve convergence

```bash
./target/release/radiosity render scenes/cornell.rad -o /tmp/scratch.png | grep solve
```

**Pass:** residual < 1e-4, iterations below the `--bounces` cap. Hitting the cap
means the solve did not converge.

---

## 6. Full check, one block

```bash
cd radiosity
cargo build --release          # expect zero warnings
cargo test --release           # expect 21 passed
./bench.sh                     # accuracy + speed + determinism gate
./target/release/radiosity render scenes/product.rad -o hero.png -w 1920 -h 1080
```

---

## 7. Reference

### CLI

```
radiosity render <scene.rad> [OPTIONS]

  -o, --output <FILE>    output PNG                  [default: out.png]
  -t, --threads <N>      worker threads              [default: all cores]
  -w, --width  <N>       override scene width
  -h, --height <N>       override scene height
      --mode <MODE>      beauty | direct | indirect | reference | normals | depth | steps
      --surfels <N>      override surfel count
      --bounces <N>      override bounce count
      --clusters <N>     transfer cluster resolution [default: 8]
      --spp <N>          samples/pixel for --mode reference
      --turntable <N>    N orbit frames from one GI solve
      --no-bvh           linear scan; verifies BVH correctness
```

```
compare <a.png> <b.png> [--diff out.png]
```

### Tuning

Both quality knobs were swept against ground truth and **neither moves SSIM**:
0.802–0.807 across 4k→90k surfels, and 0.804–0.807 across 790→6576 clusters,
while cost grows ~20×. The defaults are therefore the fast settings. Raising
them buys smoother gather interpolation at high resolution, not accuracy.

The residual error is systematic, not resolution-limited — closing it needs
hierarchical refinement of clusters that subtend a large solid angle, not more
of the same.

### Scenes

| File | Use |
|---|---|
| `scenes/cornell.rad` | Cornell box with metal and glossy materials. Main visual test. |
| `scenes/product.rad` | Open product shot. Use this for turntables. |
| `scenes/stress.rad` | 513 primitives. BVH scaling. |
| `scenes/cornell_diffuse.rad` | Partly-diffuse Cornell. |
| `scenes/bench/*.rad` | Fully Lambertian accuracy suite. |

### Troubleshooting

| Symptom | Cause |
|---|---|
| Scene renders black | Light winding. Vertices must be counter-clockwise **as seen from the emitting side**. |
| `error: no surfels generated` | No surface inside the scene bounds; check for a stray primitive far from everything else. |
| Far fewer surfels placed than requested | A single oversized primitive is inflating the world bounds, which raises the Poisson separation. Shrink it to what the camera sees — `scenes/product.rad` needed its ground slab cut from 16×16 to 9×9. |
| Turntable frames are black | Orbiting a sealed room puts the camera outside it. Use an open scene. |
| Scene too dark | Check `row-sum` in the transfer line. Well below 0.8 in a closed scene means energy calibration is failing. |
