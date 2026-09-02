## What this changes

<!-- One or two sentences. If it fixes an issue, "Fixes #123". -->

## Why

<!-- What was wrong, or what became possible. -->

---

## Checklist

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --all-targets -- -D warnings` is clean
- [ ] `cargo test --release` passes

### If this fixes a bug

- [ ] There is a test that **fails without the fix**
- [ ] The assertion message names the defect, not just the comparison
- [ ] Where the fix could be faked by a degenerate answer, the complement is tested too

### If this touches the render path

- [ ] Output is still bit-identical across thread counts:
      `interflect render scenes/cornell.rad -o a.png -t 1` and `-t 12` produce the same file
- [ ] No RNG, no parallel float reduction, no order-dependent `HashMap` iteration

### If this adds a primitive

- [ ] Distances checked against values worked out by hand
- [ ] It never **over**-reports distance (sphere tracing would step through the surface)
- [ ] Bounds enclose every interior point

### If this changes rendered output

- [ ] `./bench.sh` still passes, and any changed numbers in the README, TESTING.md
      or CHANGELOG.md are updated to match a single run

<!-- CONTRIBUTING.md explains the reasoning behind these. -->
