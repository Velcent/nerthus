# Atmosphere table generation

The renderer's `src/sky-atmosphere-data.rs` holds four orders of Eric
Bruneton's precomputed atmospheric scattering model as hexadecimal literals
in the code itself, so nothing has to be trusted as binary data. This tool
regenerates that source file from public upstream source, independently of
the editor's compiled shaders.

Run on an Apple Silicon Mac with macOS 15 or later, Swift, Python 3, and the
`glslang` and `spirv-cross` command line tools:

```sh
brew install glslang spirv-cross
python3 crates/nerthus-render/tools/sky-atmosphere/generate.py
```

For intermediate floating point tables and canonical GPU samples:

```sh
python3 crates/nerthus-render/tools/sky-atmosphere/generate.py \
  --build-directory /tmp/nerthus-atmosphere --no-package
```

The script downloads `definitions.glsl` and `functions.glsl` from
[Eric Bruneton's implementation](https://github.com/ebruneton/precomputed_atmospheric_scattering/tree/34f14e745cff948f4ca3157d1b62a445ffa7286f).
Their commit and SHA-256 hashes are pinned in `generate.py`. The upstream
copyright and redistribution terms also apply to the generated tables; see
[license-bruneton.txt](license-bruneton.txt).

The viewport configuration uses an Earth radius of 6,360 km, a 60 km atmosphere,
Rayleigh and Mie scale heights of 8 km and 1.2 km, ozone absorption, Mie
anisotropy 0.8, RGB wavelengths 680/550/440 nm, and a minimum solar zenith cosine
of cos(102 degrees). Tables are 256 x 64 for transmittance, 256 x 128 x 32 for
combined scattering, and 64 x 16 for indirect irradiance. Every intermediate
texture uses RGBA binary16, including the accumulation between scattering
orders.

Two compatibility changes are applied to the upstream equations: the earlier
solar zenith mapping uses `A = -2 * mu_s_min * bottom_radius / (H - d_min)`, and
scattering density excludes paths reflected from the ground. The combined
scattering texture stores phase-free Rayleigh and higher scattering orders in
RGB, with the red single-Mie component in alpha. The sky reconstructs the other
Mie components using the upstream combined-texture formula.

In the generated source file each texel is sixteen hexadecimal characters (the
RGBA bit patterns, sixteen bits each, red first) and a line holds eight texels
in the texture's row-major layout; the tables are listed transmittance,
combined scattering, and irradiance in that order. GPU math and texture
filtering can produce small differences between machines or shader compiler
versions. The five canonical samples cover daylight, sunset, looking away from
the sun, high altitude, and twilight; their JSON includes radiance before
exposure or white balance, plus atmospheric transmittance.