# How to test `radiosity`

Four tiers, cheapest first. Tier 1 is what you run before every commit; tier 4
is what you run before claiming a number.

| Tier | What | Time | Command |
|---|---|---|---|
| 0 | Smoke — does it run? | ~15 s | `cargo build --release && ./target/release/radiosity render scenes/product.rad -o out.png` |
| 1 | Automated suite | ~15 s | `cargo test --release` |
| 2 | Verification gates | ~1 min | §3 below |
| 3 | Accuracy benchmark | ~5 min | `./bench.sh` |
| 4 | Visual inspection | manual | §5 below |

Timings are from a 6-core / 12-thread AMD Ryzen 5 4600H, Rust 1.97.1. Shell is Git Bash / POSIX `sh`; on
PowerShell use `.\target\release\radiosity.exe` and `Get-FileHash` for `md5sum`.

---

## 1. Tier 0 — smoke test

```bash
cd radiosity
cargo build --release
./target/release/radiosity render scenes/product.rad -o out.png
```

**Pass** looks like this:

```
radiosity 0.1.0
  scene      7 objects, 2 lights, 6 materials, 3 bvh nodes
  film       640x480
  surfels    12270 placed of 20000 requested   (341 ms)
  transfer   311 clusters, 110026 links, row-sum 0.109, 10.4M sdf evals   (89 ms)
  solve      12 bounces, residual 5.37e-5, 0.3M sdf evals   (8 ms)
  shade      148 ms   15.3M sdf evals   50 evals/px
  total      586 ms
  wrote      out.png
```

The build must emit **zero warnings**. Any warning is a regression.

Open `out.png`: a chrome sphere with a drilled bowl on top, an amber rounded
box, a teal capsule and a bone torus on a grey riser. Grain anywhere in that
image is a bug — this renderer has no stochastic sampling, so it cannot produce
noise.

---

## 2. Tier 1 — automated suite

```bash
cargo test --release
```

Expected: **21 passed; 0 failed**, in under a second of test time.

Use `--release`. The debug build runs surfel generation and the transfer build
20–50× slower, and two tests will feel hung.

### What the suite covers

**Every test in `bvh`, `ltc`, `trace`, `formfactor` and `solve` reproduces the
failure signature of a real bug found during development.** They are not
coverage padding — each one failed at some point on this codebase. When one
breaks, the assertion message names the defect, not just the comparison.

| Module | Tests | Guards |
|---|---|---|
| `bvh` | 3 | Child-allocation order (traversal assumes left child is `parent + 1`); exact primitive partitioning; pruning agreeing with a linear scan over 3000 sample points. |
| `ltc` | 3 | Light winding — the form factor needs the polygon CCW as seen from the **receiver**, the DSL authors it from the **emitter**. Gets the sign wrong and every scene renders black with no other symptom. Plus a quantitative check against `A/(πd²)` and horizon clipping. |
| `trace` | 2 | Shadow-cone self-occlusion, and its complement so the fix cannot be "always return 1". |
| `formfactor` | 2 | Energy conservation (raw form factors summed to 0.213 in a sealed box whose true value is 1.0 — a scene 5× too dark); CSR structure and genuine sparsity (dense was 208 MB). |
| `solve` | 2 | No unlit interior surfel (the AO double-count drew black seams along wall junctions); solve convergence. |
| `scene` | 5 | Minimal parse, comments, planes excluded from the BVH, malformed input erroring instead of panicking, bounds not over-padded. |
| `film` | 4 | sRGB anchors and monotonicity, ACES staying inside `[0,1]`, encode buffer shape. |

### Running one test

```bash
cargo test --release closed_box_conserves_energy -- --nocapture
cargo test --release bvh::                       # whole module
```

### What the suite does **not** cover

Being explicit, because a green suite here does not mean the renderer is correct:

- **The reference path tracer is itself untested.** It is the ground truth, so
  there is nothing to check it against. It is only validated by *convergence*:
  128 spp and 512 spp agree to a luminance ratio of 0.998. A systematic bias in
  it would silently move every accuracy number.
- **Specular and GGX shading have no ground truth.** The reference integrator is
  Lambertian, so the accuracy suite deliberately excludes glossy and metal
  scenes. The specular path is only checked by eye.
- **CLI argument parsing is not unit-tested.** `parse_args` is not exposed. This
  is exactly how the `--no-bvh` flag once shipped parsed-but-never-applied,
  making a correctness gate pass vacuously — hence the shell gate in §3.
- **The turntable camera orbit is not tested.**
- **Surfel density uniformity** is only checked indirectly, via the energy test.
- **No fuzzing or property-based testing.**

Several of these are now covered *outside* the unit suite, by the CI gates in
`.github/workflows/ci.yml` — Linux/Windows/macOS builds, the `--no-bvh` gate,
accuracy against ground truth, and malformed-input handling. What remains
genuinely untested is the reference integrator, specular shading, and the
turntable orbit.

---

## 3. Tier 2 — verification gates

Integration-level; these cannot be expressed as unit tests.

### 3.1 Determinism across thread counts

The headline claim. If this fails, a float reduction is summing in
nondeterministic order somewhere.

```bash
for t in 1 3 8 12; do
  ./target/release/radiosity render scenes/cornell.rad -o det_$t.png -w 300 -h 300 -t $t
done
md5sum det_*.png
```

**Pass:** all four hashes identical.

### 3.2 Reproducibility across runs

```bash
for i in 1 2 3; do
  ./target/release/radiosity render scenes/cornell.rad -o rep_$i.png -w 300 -h 300
done
md5sum rep_*.png
```

**Pass:** all three identical.

### 3.3 The `--no-bvh` flag is actually wired

This gate exists because the flag was once parsed and never applied, so the BVH
correctness check passed with identical timings and proved nothing. Timing is the
only observable difference — both paths must produce the same pixels.

```bash
./target/release/radiosity render scenes/stress.rad -o b.png --mode normals | grep shade
./target/release/radiosity render scenes/stress.rad -o l.png --mode normals --no-bvh | grep shade
./target/release/compare b.png l.png
```

**Pass:** linear >2× slower (measured **8.2×** — 136 ms vs 1111 ms on 513
primitives) **and** SSIM 1.000000.

A couple of pixels may differ at silhouette edges (measured: 2 in 360k, max
7/255). Not a pruning error — LLVM makes different auto-vectorisation and
FMA-contraction choices for the two loops, so a sphere-tracing ray sitting
exactly on the hit epsilon can fall either way.

### 3.4 Energy conservation

```bash
./target/release/radiosity render scenes/cornell.rad -o /tmp/scratch.png --surfels 16000 | grep transfer
```

**The pass threshold depends on scene topology** — this trips people up:

| Scene | Expected `row-sum` | Why |
|---|---|---|
| `cornell.rad` (sealed box) | **≈0.82** | Every direction from an interior surface hits geometry, so the sum must approach 1.0. |
| `product.rad` (open) | **≈0.11** | Most of each hemisphere is empty sky. Low is *correct*. |
| `stress.rad` (open) | **≈0.15** | Same. |

So `row-sum` near zero is only a failure in a **closed** scene. It read 0.213
there before the hemisphere-closure calibration, leaving the scene ~5× too dark.

### 3.5 Solve convergence

```bash
./target/release/radiosity render scenes/cornell.rad -o /tmp/scratch.png | grep solve
```

**Pass:** residual < 1e-4 and iterations below the `--bounces` cap. Hitting the
cap means the solve did not converge.

### 3.6 Malformed input

Every case must produce a diagnostic and exit 1 — never a panic, never a
zero-byte PNG. Covered by `scene::tests::malformed_input_errors_instead_of_panicking`,
but worth spot-checking against the binary:

```bash
echo 'wibble { }' > /tmp/bad.rad
./target/release/radiosity render /tmp/bad.rad -o /tmp/b.png; echo "exit: $?"
```

**Pass:** `error: /tmp/bad.rad: unknown block \`wibble\`` and exit 1.

---

## 4. Tier 3 — accuracy benchmark

```bash
./bench.sh                    # 384 spp, 260 px — about 5 minutes
SPP=1024 RES=400 ./bench.sh   # tighter reference, much slower
```

Renders every scene in `scenes/bench/` twice — once with the solver, once with
the built-in path tracer as ground truth — then reports SSIM, speedup and energy
ratio, finishing with the determinism gate.

Expected, within a few percent:

```
SCENE                     OURS_MS     REF_MS   SPEEDUP     SSIM     LUMA
box_only                     1461      31277     21.4x 0.838243    1.065
cornell                      1358      28568     21.0x 0.804378    1.047
high_albedo                  1502      32477     21.6x 0.818704    1.046
sphere_only                  1389      30235     21.8x 0.849417    1.088
```

### Reading the SSIM column — the ceiling is 0.89, not 1.0

The reference is stochastic, and SSIM penalises its residual noise even against
a *perfect* image. Measure the ceiling yourself:

```bash
./target/release/radiosity render scenes/bench/cornell.rad -o r128.png --mode reference --spp 128
./target/release/radiosity render scenes/bench/cornell.rad -o r512.png --mode reference --spp 512
./target/release/compare r128.png r512.png
```

Two references of the *same* scene, differing only in sample count, score
**0.892** against each other. So ~0.83 measured sits roughly 93% of the way to
the achievable ceiling — quote it that way, not as "83% accurate".

The `LUMA` column is the more honest accuracy signal: it is a straight energy
ratio, unaffected by noise. Ours lands within 4.6–8.8% of ground truth.

Scenes in `scenes/bench/` are fully Lambertian by necessity. A glossy scene
would measure the gap between two different BSDFs rather than the accuracy of
the light transport.

### Do not tune against this benchmark

Both quality knobs were swept and **neither moves SSIM**: 0.802–0.807 across
4k→90k surfels, 0.804–0.807 across 790→6576 clusters, while cost grows ~20×.
The residual error is systematic, not resolution-limited. If a change improves
SSIM by a fraction of a percent, suspect overfitting before celebrating.

---

## 5. Tier 4 — visual inspection

Some defects are invisible to SSIM but obvious to the eye. Render the Cornell
box and compare modes:

```bash
./target/release/radiosity render scenes/cornell.rad -o direct.png --mode direct
./target/release/radiosity render scenes/cornell.rad -o beauty.png --mode beauty
./target/release/compare direct.png beauty.png --diff gi_only.png
```

`direct.png` should have **fully black shadows** and an **unlit block face**.
Everything the eye reads as fill light in `beauty.png` comes from the solve. If
`direct.png` already looks filled in, indirect light is leaking into the direct
path.

### Defect catalogue

Each of these was a real failure state during development. Knowing what they
look like is faster than bisecting.

| What you see | Cause |
|---|---|
| **Entirely black image**, geometry visible in `--mode normals` | Light winding reversed. Form factors go negative and clamp to zero. |
| **Concentric contour rings** on lit walls | Shadow-cone penumbra sampled too coarsely, or the closest-approach correction missing. |
| **Black seams along wall junctions** | AO multiplied into the indirect term. The transfer matrix already carries visibility, and AO tends to zero exactly where GI should get brighter. |
| **Whole scene uniformly too dark**, `row-sum` low in a closed scene | Energy calibration failing. |
| **Metal or mirror renders pure black** | Specular indirect path missing — a metal has no diffuse lobe, so it has nothing else to show. |
| **Washed-out, desaturated walls** | Over-bright indirect; ACES desaturates highlights, so an energy error shows up as loss of colour before it shows as brightness. |
| **Blotchy patches on curved surfaces** | Surfel gather radius too large relative to density. |
| **Grain / speckle anywhere** | Should be impossible. Something stochastic entered the render path. |
| **Turntable frames black** | Camera orbiting outside a sealed room. Use an open scene. |
| **Far fewer surfels placed than requested** | One oversized primitive inflating world bounds, which raises the Poisson separation. `product.rad` needed its ground slab cut from 16×16 to 9×9. |

### Debug modes

```bash
./target/release/radiosity render scenes/cornell.rad -o n.png --mode normals   # geometry & normals
./target/release/radiosity render scenes/cornell.rad -o d.png --mode depth
./target/release/radiosity render scenes/cornell.rad -o s.png --mode steps     # green cheap, red expensive
./target/release/radiosity render scenes/cornell.rad -o i.png --mode indirect  # GI in isolation
```

`--mode normals` is the first thing to check when an image looks wrong: it
isolates geometry from all lighting. If normals look right, the bug is in
transport.

---

## 6. Full check, one block

```bash
cd radiosity
cargo build --release     # zero warnings
cargo test --release      # 21 passed
./bench.sh                # accuracy + speed + determinism gate
```

---

## 7. Adding a test for a new bug

The convention in this repo: **a bug fix ships with a test that fails before the
fix**. Write the test first and watch it fail — a test written after the fix
often asserts the wrong thing.

1. Reproduce the failure in the smallest scene that shows it. Several tests here
   embed a scene as a `const &str` and call `scene::parse`.
2. Put the test in the module that owns the defect, in `#[cfg(test)] mod tests`.
3. Make the assertion message name the *defect*, not the comparison. Compare
   `assert!(vis > 0.9)` against the actual message used:
   `"floor directly beneath an unobstructed light is only {:.3} lit — the shadow
   cone is reading the ceiling plane it runs parallel to"`.
4. Where the fix could be faked by a degenerate answer, add the complement.
   `real_occluder_still_casts_shadow` exists so the shadow fix cannot be
   "always return 1".
5. Verify it fails on the unfixed code, then apply the fix.

That process is not theoretical here: writing
`unoccluded_floor_under_flush_light_is_not_self_shadowed` found a live bug the
existing renders had not surfaced — the closest-approach correction is only
valid while a ray *approaches* a surface, and a ray moving directly away from a
plane doubles its distance each step, making the correction collapse visibility
to zero.
