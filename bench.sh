#!/usr/bin/env bash
# Accuracy + performance benchmark.
#
# Renders each scene twice: once with the radiosity solver, once with the
# built-in brute-force path tracer as ground truth, then reports SSIM and the
# speed ratio. Every number in the README comes from this script.
#
# Note the SSIM ceiling reported at the end: the reference is stochastic, and
# SSIM penalises its residual noise even against a perfect image. Read the
# per-scene scores against that ceiling, not against 1.0.
set -u

BIN=./target/release/interflect
CMP=./target/release/compare
SPP=${SPP:-512}
RES=${RES:-300}
OUT=bench_out
mkdir -p "$OUT"

if [ ! -x "$BIN" ]; then
  echo "build first: cargo build --release" >&2
  exit 1
fi

printf '%-22s %10s %10s %9s %8s %8s\n' SCENE OURS_MS REF_MS SPEEDUP SSIM LUMA
printf '%.0s-' {1..72}; printf '\n'

for scene in scenes/bench/*.rad; do
  name=$(basename "$scene" .rad)
  # Every scene here is fully Lambertian (roughness 1.0, metallic 0). That is
  # required, not incidental: the reference integrator is diffuse-only, so a
  # glossy or metallic scene would be measuring the gap between two different
  # BSDFs rather than the accuracy of the light transport.
  ours="$OUT/${name}_ours.png"
  ref="$OUT/${name}_ref.png"

  t0=$(date +%s%N)
  $BIN render "$scene" -o "$ours" -w $RES -h $RES >/dev/null 2>&1 || continue
  t1=$(date +%s%N)
  ours_ms=$(( (t1 - t0) / 1000000 ))

  t0=$(date +%s%N)
  $BIN render "$scene" -o "$ref" --mode reference --spp $SPP -w $RES -h $RES >/dev/null 2>&1 || continue
  t1=$(date +%s%N)
  ref_ms=$(( (t1 - t0) / 1000000 ))

  out=$($CMP "$ours" "$ref")
  ssim=$(echo "$out" | grep SSIM | awk '{print $2}')
  luma=$(echo "$out" | grep 'mean luma' | sed 's/.*ratio \([0-9.]*\)).*/\1/')
  speedup=$(awk "BEGIN{printf \"%.1fx\", $ref_ms/$ours_ms}")

  printf '%-22s %10s %10s %9s %8s %8s\n' "$name" "$ours_ms" "$ref_ms" "$speedup" "$ssim" "$luma"
done

echo
echo "Perf-only scenes (glossy/metal, excluded from the accuracy table):"
for scene in scenes/cornell.rad scenes/product.rad scenes/stress.rad; do
  name=$(basename "$scene" .rad)
  t0=$(date +%s%N)
  $BIN render "$scene" -o "$OUT/${name}.png" -w $RES -h $RES >/dev/null 2>&1
  t1=$(date +%s%N)
  printf '  %-20s %6s ms
' "$name" $(( (t1 - t0) / 1000000 ))
done

echo
echo "Determinism gate:"
for t in 1 4 12; do
  $BIN render scenes/cornell.rad -o "$OUT/det_$t.png" -w 200 -h 200 -t $t >/dev/null 2>&1
done
if [ "$(md5sum < "$OUT/det_1.png")" = "$(md5sum < "$OUT/det_12.png")" ] && \
   [ "$(md5sum < "$OUT/det_4.png")" = "$(md5sum < "$OUT/det_12.png")" ]; then
  echo "  PASS  bit-identical across 1, 4 and 12 threads"
else
  echo "  FAIL  output varies with thread count"
  exit 1
fi
