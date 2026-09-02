# Contributing

Contributions are welcome. This file covers the two rules that are specific to
this project; everything else is ordinary.

## Setup

```bash
git clone https://github.com/Cherie05/interflect
cd interflect
cargo build --release
cargo test --release      # 25 tests, ~15 s
```

Use `--release`. In a debug build the surfel and transfer stages run 20–50×
slower and two tests will look like they have hung.

## Before opening a pull request

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --release
```

CI runs all three plus eight correctness gates on Linux, Windows and macOS. It
gates things the unit tests cannot reach — determinism across thread counts,
energy conservation, `--no-bvh` correctness, accuracy against the built-in path
tracer. [TESTING.md](TESTING.md) explains each one.

---

## Rule 1 — a bug fix ships with a test that fails before the fix

Write the test first and watch it fail. A test written after the fix often
asserts the wrong thing, and this is not hypothetical here: the test written for
the shadow-cone bug immediately exposed a *second*, unrelated defect that the
existing renders had never surfaced.

Three things make these tests useful:

**Put it in the module that owns the defect.** Several tests embed a small scene
as a `const &str` and call `scene::parse`, which keeps the reproduction next to
the code.

**Make the assertion message name the defect, not the comparison.** Compare a
bare `assert!(vis > 0.9)` with what is actually in `trace.rs`:

> `"floor directly beneath an unobstructed light is only {:.3} lit — the shadow
> cone is reading the ceiling plane it runs parallel to"`

The second one tells the next person what broke.

**Where a fix could be faked by a degenerate answer, add the complement.**
`real_occluder_still_casts_shadow` exists so the shadow fix cannot quietly
become "always return 1".

## Rule 2 — determinism is not negotiable

Output must be bit-identical across thread counts and across runs. That is the
project's headline claim and CI enforces it. It constrains how you write code:

- **No RNG in the render path.** Halton sequences with fixed indices instead.
- **No parallel float reductions.** Summation order varies between runs, and the
  last bits drift. Cluster aggregation in `solve.rs` is deliberately serial for
  this reason; it is O(N) against an O(N·C) gather, so it costs nothing.
- **Jacobi, not Gauss-Seidel.** Jacobi reads only the previous iterate, so rows
  update in any order and give identical results.
- **Never iterate a `HashMap` where order affects output.** Sort the keys first.
  `formfactor.rs` does this for exactly this reason.

If a change makes the determinism gate fail, the change is wrong — not the gate.

---

## Adding a primitive

A distance field that lies produces subtly wrong images rather than an obvious
failure, so new primitives need four things. `sdf.rs` has the pattern:

1. Distances checked against values worked out by hand.
2. A case that distinguishes it from a neighbouring shape — a cylinder's flat
   cap corner reports `sqrt(2)` where a capsule would not.
3. **It must never over-report distance.** Sphere tracing steps by the reported
   value; a field that claims more clearance than it has lets rays walk through
   surfaces. The existing test probes 4,000 points per primitive.
4. Bounds that enclose every interior point, or the BVH prunes away real hits.

## Adding a scene

Drop a `.rad` file in `scenes/`. CI renders every scene in the tree, so a broken
one fails the build. Scenes in `scenes/bench/` must be **fully Lambertian** —
the reference integrator is diffuse-only, so a glossy scene there would measure
the gap between two BSDFs rather than the accuracy of the light transport.

## Good first issues

- A roughness-driven cone trace for blurry reflections (currently the mirror
  direction is attenuated rather than blurred).
- Skip surfels on the outside of wall slabs — they see nothing and consume a
  large share of the budget.
- Fitted LTC tables to replace the representative-point specular approximation.
- A `--watch` mode that re-renders when the scene file changes.
- Linux arm64 release target.

## Where things are

| File | Contents |
|---|---|
| `sdf.rs`, `bvh.rs` | primitives, CSG, nearest-distance acceleration |
| `trace.rs` | sphere tracing, cone shadows, hemisphere closure |
| `surfel.rs` | **meshless surface sampling** — the novel part |
| `formfactor.rs` | **transfer matrix** — the novel part |
| `solve.rs` | **radiosity iteration** — the novel part |
| `ltc.rs`, `shade.rs` | analytic area lights, BSDFs |
| `reference.rs` | path tracer, used only as benchmark ground truth |

[CREDITS.md](CREDITS.md) attributes the published work each component comes
from. If you port an algorithm from a paper or an article, add it there.

## Reporting a bug

Include the `.rad` scene that reproduces it and the renderer's stdout — the
`surfels` / `transfer` / `solve` lines carry the diagnostics (`row-sum` and
`residual` in particular). A rendered PNG helps. [TESTING.md](TESTING.md) has a
catalogue of what each failure mode looks like on screen.
