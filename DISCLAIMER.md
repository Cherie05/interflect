# Disclaimer

The legally operative terms are in [LICENSE](LICENSE). This file restates them
in plain language and adds context specific to what this software claims. It
does not replace, extend or narrow the licence — if the two ever disagree, the
licence governs.

## No warranty

Interflect is provided **as is**, with no warranty of any kind. It may contain
defects. It may produce incorrect images. It may fail on your hardware, your
operating system, or your scene.

The author is not liable for any damage, loss or cost arising from its use.
This is the standard MIT position and it is deliberate: the software is given
away for free, and it comes with no promises attached.

## About the performance and accuracy figures

Every number published in the README and on the project website was measured on
**one machine** — a 6-core / 12-thread AMD Ryzen 5 4600H — on **four small
benchmark scenes**, against this project's own built-in path tracer.

They are honest measurements, and `./bench.sh` reproduces them. They are **not**
a prediction of what you will see. Results depend on your CPU, your scene, your
resolution and your settings, and the speedup figures compare against *this*
reference integrator, not against Blender Cycles, PBRT, or any production
renderer.

Read the accuracy figures with the caveat stated alongside them: SSIM is scored
against a stochastic reference whose own noise depresses the metric, so the
practical ceiling is about 0.892, not 1.0.

## Not fit for safety-critical use

Interflect is a rendering tool for pictures. It is **not** validated for, and
must not be relied upon for, any purpose where an incorrect result could cause
harm or loss — including lighting design compliance, photometric certification,
architectural daylight analysis for regulatory submission, medical imaging, or
anything safety-critical.

It is not a photometrically calibrated simulator. It approximates light
transport well enough to produce convincing images, and no further claim is
made. [Radiance](https://www.radiance-online.org/) is the tool for validated
lighting analysis.

## Known limitations

The README lists what the renderer deliberately does not do — no caustics, no
refraction, no participating media, no triangle meshes, reflections attenuated
rather than blurred, and a known banding artefact in penumbrae. These are
documented scope decisions, not undisclosed defects.

## Security

Interflect opens no network connections, executes nothing it reads, and needs no
elevated privileges. The realistic risk is a malicious scene file, and
[SECURITY.md](SECURITY.md) describes the threat model and how to report an
issue. As with any software, run untrusted input at your own risk.

## Third-party work

[NOTICE.md](NOTICE.md) records the published algorithms this renderer
implements and the licences of its dependencies. If you believe attribution is
wrong or missing, open an issue — it will be corrected.

## Trade marks

"Interflect" is used here only as the name of this project. No trade mark claim
is asserted, and no affiliation with any similarly named business is implied.
