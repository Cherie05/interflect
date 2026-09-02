# Credits

Interflect takes its name from **interreflection** — the term for light bouncing
between surfaces, which is precisely what this renderer solves and what a path
tracer spends millions of random samples approximating.

Almost nothing here is new. The one genuinely novel piece is the combination:
meshless surfel placement on an SDF, feeding a classical radiosity solve. Every
component it is built from belongs to someone else, and they are listed below.

---

## The method this project revives

**Cindy M. Goral, Kenneth E. Torrance, Donald P. Greenberg, Bennett Battaile**
(1984). *Modeling the Interaction of Light between Diffuse Surfaces.*
SIGGRAPH Computer Graphics 18(3), 213–222.

The origin of radiosity in computer graphics. It borrowed the method from
thermal engineering and was the first to reproduce diffuse object-to-object
reflection — the colour bleeding that this renderer's Cornell box exists to
demonstrate.

> [SIGGRAPH History Archives](https://history.siggraph.org/learning/modeling-the-interaction-of-light-between-diffuse-surfaces-by-goral-torrance-greenberg-and-battaile/)
> · [paper (PDF)](http://www0.cs.ucl.ac.uk/research/vr/Projects/VLF/vlfpapers/radiosity/Goral__Modeling_the_Interaction_of_Light_Between_Diffuse_Surfaces.pdf)

Bennett Battaile is the least-cited of the four and deserves the mention.

## Rendering the geometry

**John C. Hart** (1996). *Sphere tracing: a geometric method for the antialiased
ray tracing of implicit surfaces.* The Visual Computer 12, 527–545.

Every ray in `trace.rs` is a sphere trace: step along the ray by exactly the
distance to the nearest surface, which is guaranteed never to overshoot. Hart's
insight was that this needs only a bound on the magnitude of the derivative —
not the derivative itself — which is why it survives creased and pathological
surfaces where other implicit-surface methods fail. It is also what makes the
cone-traced soft shadows here possible: sphere tracing approximates cone tracing
almost for free.

> [The Visual Computer](https://link.springer.com/article/10.1007/s003710050084)
> · [paper (PDF)](https://graphics.stanford.edu/courses/cs348b-20-spring-content/uploads/hart.pdf)

## Transferring the light

**Wilhelm Nusselt** — the Nusselt analog.

The form factor between two patches equals the area you get by projecting one
patch radially onto a unit hemisphere at the other, then projecting that
orthographically down onto the hemisphere's base, as a fraction of the base
area. `formfactor.rs` uses the disc-to-disc form derived from it. A German heat
transfer physicist, working on thermal radiation long before any of this was
about pictures.

> [form factor derivation](https://education.siggraph.org/static/HyperGraph/radiosity/overview_2.htm)

## Area lights

**Johann Heinrich Lambert** (1760). *Photometria.*

The cosine-weighted solid angle subtended by a polygon has a closed form. It is
what `ltc.rs` evaluates, and it is why the direct lighting here needs no samples
at all — a path tracer spends hundreds of shadow rays per pixel approximating
what Lambert solved analytically 265 years ago.

**Eric Heitz, Jonathan Dupuy, Stephen Hill, David Neubelt** (2016). *Real-Time
Polygonal-Light Shading with Linearly Transformed Cosines.* SIGGRAPH.

The modern generalisation. The diffuse case this renderer uses is LTC with the
identity transform; the fitted matrices for GGX are the drop-in upgrade noted in
`ltc.rs`.

**Brian Karis** (2013). *Real Shading in Unreal Engine 4.*

The representative-point approximation used for the specular lobe against area
lights.

## Surfaces

**Bruce Walter, Stephen Marschner, Hongsong Li, Kenneth Torrance** (2007).
*Microfacet Models for Refraction through Rough Surfaces.* EGSR.
The GGX / Trowbridge-Reitz distribution in `shade.rs`.

**Christophe Schlick** (1994). *An Inexpensive BRDF Model for Physically-based
Rendering.* Eurographics. The Fresnel approximation.

**Eric Heitz** (2014). *Understanding the Masking-Shadowing Function in
Microfacet-Based BRDFs.* The height-correlated Smith visibility term.

## Sampling and structure

**John Halton** (1960). The low-discrepancy sequence that places every surfel.
Choosing it over an RNG is what makes the output bit-identical across runs and
thread counts.

**Robert Bridson** (2007). *Fast Poisson Disk Sampling in Arbitrary Dimensions.*
The rejection strategy that keeps surfel spacing even.

**Ingo Wald, Solomon Boulos, Peter Shirley** and others — the binned surface-area
heuristic BVH. Adapted here for nearest-distance queries rather than ray
intersection, since sphere tracing never asks "what does this ray hit?", only
"how far is the nearest surface?"

**Tom Duff, James Burgess, Per Christensen, Christophe Hery, Andrew Kensler,
Max Liani, Ryusuke Villemin** (2017). *Building an Orthonormal Basis, Revisited.*
JCGT. The branchless tangent frame in `ltc.rs`.

## Practice

**Inigo Quilez** — the distance functions for the rounded box, capsule, torus,
capped cylinder and capped cone, and the soft-shadow and ambient-occlusion
formulations. Published freely over two decades at
[iquilezles.org](https://iquilezles.org/articles/distfunctions/), and the reason
SDF rendering is accessible at all.

**Krzysztof Narkowicz** (2015) — the analytic ACES tone-curve fit in `film.rs`.

**Matt Pharr, Wenzel Jakob, Greg Humphreys** — *Physically Based Rendering.*
The reference path tracer in `reference.rs` follows its treatment of next-event
estimation and Russian roulette.

**The Cornell Box** — Cornell University Program of Computer Graphics, 1984.
Still the measuring stick for global illumination, and still the first scene
worth testing against.

---

## On the name

*Interreflection* is the established term for reciprocal reflection between
surfaces. It is the entire subject of this renderer: the difference between the
`--mode direct` and `--mode beauty` images is exactly the interreflected light,
and the radiosity solve exists to compute it without sampling.

The project was called `radiosity` during development. That was a bad name for
the same reason calling a path tracer "raytracer" would be: it names the
category, not the thing.

Checked before adopting `interflect`: no company, no registered trademark, and
no package on crates.io, npm or PyPI. The only existing use found anywhere was
an unrelated design podcast. It is a coinage, not a dictionary word — the real
word is *interreflection*, which was too long to type.
