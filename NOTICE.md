# Notice

Interflect is licensed under the [MIT License](LICENSE). This file records the
provenance of the algorithms it implements, and the licences of the crates it
depends on.

## Algorithms

Interflect implements published algorithms from the computer graphics and heat
transfer literature. **Every implementation in this repository was written from
the published mathematical description**, not copied from another codebase. The
underlying mathematics — a form factor, a distance function, a sampling
sequence — is not itself subject to copyright; the expression of it in this
repository is original work by the author and is covered by the MIT licence
above.

Where a specific formulation is due to a named person, the source file names
them at the point of use, and [CREDITS.md](CREDITS.md) lists all seventeen with
citations. The principal sources:

| Source | Used for |
|---|---|
| Goral, Torrance, Greenberg & Battaile (1984) | the radiosity formulation |
| Hart, *Sphere Tracing* (1996) | ray marching against distance fields |
| Nusselt (1928) | the disc-to-disc form factor |
| Lambert, *Photometria* (1760) | closed-form polygon irradiance |
| Heitz, Dupuy, Hill & Neubelt (2016) | linearly transformed cosines |
| Walter, Marschner, Li & Torrance (2007) | the GGX microfacet distribution |
| Schlick (1994) | the Fresnel approximation |
| Halton (1960) | the low-discrepancy sequence |
| Bridson (2007) | Poisson-disc rejection sampling |
| Duff et al. (2017) | branchless orthonormal basis construction |
| Inigo Quilez, [iquilezles.org](https://iquilezles.org/articles/distfunctions/) | signed distance function formulations |
| Narkowicz (2015) | the analytic ACES tone-curve fit |

If you believe any part of this repository reproduces your work in a way that
requires different attribution or a different licence, please open an issue or
email <arunvpp24@gmail.com>. It will be corrected.

## Dependencies

Interflect has three direct dependencies, all permissively licensed:

| Crate | Licence |
|---|---|
| [`glam`](https://crates.io/crates/glam) | MIT OR Apache-2.0 |
| [`rayon`](https://crates.io/crates/rayon) | MIT OR Apache-2.0 |
| [`image`](https://crates.io/crates/image) | MIT OR Apache-2.0 |

Their transitive dependencies (24 crates in total) are likewise MIT, Apache-2.0
or dual-licensed. Run `cargo tree` for the full graph and `cargo license` for a
per-crate breakdown.

Distributing a compiled Interflect binary means distributing those crates'
compiled code. Their licences require their copyright notices to be preserved;
the release archives include this file and `LICENSE` for that reason.

## Test scenes

The Cornell Box (`scenes/cornell.rad`) reproduces the geometry of the standard
test scene from the Cornell University Program of Computer Graphics (1984). The
scene is a de facto standard used across the field for comparing global
illumination; the `.rad` file here is an original description of that geometry
written for this renderer's own format.
