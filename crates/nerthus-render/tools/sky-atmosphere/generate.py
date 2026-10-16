#!/usr/bin/env python3
"""Generate four-order atmospheric scattering tables on an Apple Metal GPU."""

import argparse
from array import array
import hashlib
import json
import math
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
import urllib.request

UPSTREAM_COMMIT = "34f14e745cff948f4ca3157d1b62a445ffa7286f"
UPSTREAM_URL = (
    "https://raw.githubusercontent.com/ebruneton/"
    f"precomputed_atmospheric_scattering/{UPSTREAM_COMMIT}/atmosphere/"
)
UPSTREAM_HASHES = {
    "definitions.glsl": "fb7579bc25f7b22b19aaa479c2a009eb70875216062a58b329d1ca2f01b8794b",
    "functions.glsl": "fae5b16bdd2b5a1d1a8b423c5b76973984eaf9b7514c349fe74b1d44d0c9f498",
}
REPOSITORY_ROOT = Path(__file__).resolve().parents[4]
DEFAULT_OUTPUT = (
    REPOSITORY_ROOT / "crates/nerthus-render/src/sky-atmosphere-data.rs"
)

SOLAR_IRRADIANCE = [1.474, 1.8504, 1.91198]
OZONE_ABSORPTION = [1.209e-25, 3.5e-25, 1.582e-26]
WAVELENGTHS = [680.0, 550.0, 440.0]


def runCommand(arguments):
    subprocess.run(arguments, check=True)


def fetchSource(name, build_directory):
    source_file = build_directory / name
    if not source_file.exists():
        with urllib.request.urlopen(UPSTREAM_URL + name) as response:
            source_file.write_bytes(response.read())
    source_bytes = source_file.read_bytes()
    actual_hash = hashlib.sha256(source_bytes).hexdigest()
    if actual_hash != UPSTREAM_HASHES[name]:
        raise ValueError(f"Unexpected source contents: {source_file}")
    return source_bytes.decode()


def patchFunctions(source):
    current_mapping = (
        "  Length D = DistanceToTopAtmosphereBoundary(\n"
        "      atmosphere, atmosphere.bottom_radius, atmosphere.mu_s_min);\n"
        "  Number A = (D - d_min) / (d_max - d_min);"
    )
    previous_mapping = (
        "  Number A = -2.0 * atmosphere.mu_s_min * atmosphere.bottom_radius /\n"
        "      (d_max - d_min);"
    )
    if source.count(current_mapping) != 2:
        raise ValueError("Upstream scattering coordinate mapping changed")
    source = source.replace(current_mapping, previous_mapping)

    # The viewport atmosphere excludes paths whose final bounce is on the ground.
    ground_start = source.index(
        "    // The distance and transmittance to the ground only depend on theta"
    )
    ground_end = source.index("    for (int m =", ground_start)
    source = source[:ground_start] + source[ground_end:]
    ground_start = source.index("      // and of the contribution from the light paths")
    ground_end = source.index("      // The radiance finally scattered", ground_start)
    source = source[:ground_start] + source[ground_end:]
    return source


def vectorLiteral(values):
    components = [format(value, ".17g") for value in values]
    return "vec3(" + ", ".join(components) + ")"


def makeHeader(definitions, functions):
    rayleigh = [6.2031e-7 * (wavelength * 0.001) ** -4 * 1000.0
                for wavelength in WAVELENGTHS]
    absorption = [300.0 * 2.687e20 / 15000.0 * value * 1000.0
                  for value in OZONE_ABSORPTION]
    atmosphere = f"""
AtmosphereParameters makeAtmosphere() {{ return AtmosphereParameters(
    {vectorLiteral(SOLAR_IRRADIANCE)}, 0.004675, model_bottom_radius, model_top_radius,
    DensityProfile(DensityProfileLayer[2](
        DensityProfileLayer(0.0, 0.0, 0.0, 0.0, 0.0),
        DensityProfileLayer(0.0, 1.0, -0.125, 0.0, 0.0))),
    {vectorLiteral(rayleigh)},
    DensityProfile(DensityProfileLayer[2](
        DensityProfileLayer(0.0, 0.0, 0.0, 0.0, 0.0),
        DensityProfileLayer(0.0, 1.0, {-1000.0 / 1200.0:.17g}, 0.0, 0.0))),
    vec3(0.003996), vec3(0.00444), 0.8,
    DensityProfile(DensityProfileLayer[2](
        DensityProfileLayer(25.0, 0.0, 0.0, {1.0 / 15.0:.17g}, {-2.0 / 3.0:.17g}),
        DensityProfileLayer(0.0, 0.0, 0.0, {-1.0 / 15.0:.17g}, {8.0 / 3.0:.17g}))),
    {vectorLiteral(absorption)}, vec3(0.1), {math.cos(math.radians(102.0)):.17g}); }}
#define ATMOSPHERE makeAtmosphere()
const vec3 SKY_SPECTRAL_RADIANCE_TO_LUMINANCE = vec3(1.0);
const vec3 SUN_SPECTRAL_RADIANCE_TO_LUMINANCE = vec3(1.0);
"""
    prefix = """#version 450
#define IN(x) const in x
#define OUT(x) out x
#define TEMPLATE(x)
#define TEMPLATE_ARGUMENT(x)
#define assert(x)
#define COMBINED_SCATTERING_TEXTURES
layout(binding = 0, std140) uniform Parameters {
    int layer;
    int scatteringOrder;
    float model_bottom_radius;
    float model_top_radius;
};
const int TRANSMITTANCE_TEXTURE_WIDTH = 256;
const int TRANSMITTANCE_TEXTURE_HEIGHT = 64;
const int SCATTERING_TEXTURE_R_SIZE = 32;
const int SCATTERING_TEXTURE_MU_SIZE = 128;
const int SCATTERING_TEXTURE_MU_S_SIZE = 32;
const int SCATTERING_TEXTURE_NU_SIZE = 8;
const int IRRADIANCE_TEXTURE_WIDTH = 64;
const int IRRADIANCE_TEXTURE_HEIGHT = 16;
"""
    resources = """
layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;
layout(binding = 1) uniform sampler2D transmittanceRead;
layout(binding = 2) uniform sampler3D singleRayleighScatteringRead;
layout(binding = 3) uniform sampler3D singleMieScatteringRead;
layout(binding = 4) uniform sampler3D multipleScatteringRead;
layout(binding = 5) uniform sampler2D irradianceRead;
layout(binding = 6) uniform sampler3D scatteringRead;
layout(binding = 8) uniform sampler3D deltaScatteringDensityRead;
layout(binding = 16, rgba16f) writeonly uniform image2D transmittanceWrite;
layout(binding = 17, rgba16f) writeonly uniform image2D deltaIrradianceWrite;
layout(binding = 18, rgba16f) writeonly uniform image2D irradianceWrite;
layout(binding = 19, rgba16f) writeonly uniform image3D deltaRayleighScatteringWrite;
layout(binding = 20, rgba16f) writeonly uniform image3D deltaMieScatteringWrite;
layout(binding = 21, rgba16f) writeonly uniform image3D scatteringWrite;
layout(binding = 22, rgba16f) writeonly uniform image3D singleMieScatteringWrite;
layout(binding = 23, rgba16f) writeonly uniform image3D deltaScatteringDensityWrite;
layout(binding = 24, rgba16f) writeonly uniform image3D deltaMultipleScatteringWrite;
layout(binding = 25, rgba32f) writeonly uniform image2D sampleWrite;
"""
    return prefix + definitions + atmosphere + functions + resources


def makeKernels():
    return {
        "ComputeSkySamples": """
const vec4 rays[5] = vec4[5](
    vec4(1.0, 60.0, 90.0, 0.0),
    vec4(1.0, 5.0, 2.0, 0.0),
    vec4(1.0, 5.0, 20.0, 180.0),
    vec4(40.0, 33.0, 0.0, 90.0),
    vec4(2.0, -5.0, 10.0, 0.0));
if (texel.x >= 5 || texel.y != 0) return;
vec4 ray = rays[texel.x];
float sun_elevation = radians(ray.y);
float view_elevation = radians(ray.z);
float azimuth = radians(ray.w);
vec3 camera = vec3(0.0, model_bottom_radius + ray.x, 0.0);
vec3 view_direction = vec3(sin(azimuth) * cos(view_elevation),
    sin(view_elevation), cos(azimuth) * cos(view_elevation));
vec3 sun_direction = vec3(0.0, sin(sun_elevation), cos(sun_elevation));
vec3 transmittance;
vec3 radiance = GetSkyRadiance(ATMOSPHERE, transmittanceRead,
    scatteringRead, singleMieScatteringRead, camera, view_direction,
    0.0, sun_direction, transmittance);
imageStore(sampleWrite, ivec2(texel.x, 0), vec4(radiance, 1.0));
imageStore(sampleWrite, ivec2(texel.x, 1), vec4(transmittance, 1.0));
""",
        "ComputeTransmittance": """
vec3 value = ComputeTransmittanceToTopAtmosphereBoundaryTexture(ATMOSPHERE, coords.xy);
imageStore(transmittanceWrite, texel.xy, vec4(value, 1.0));
""",
        "ComputeDirectIrradiance": """
vec3 value = ComputeDirectIrradianceTexture(ATMOSPHERE, transmittanceRead, coords.xy);
imageStore(deltaIrradianceWrite, texel.xy, vec4(value, 1.0));
imageStore(irradianceWrite, texel.xy, vec4(0.0, 0.0, 0.0, 1.0));
""",
        "ComputeSingleScattering": """
vec3 rayleigh;
vec3 mie;
ComputeSingleScatteringTexture(ATMOSPHERE, transmittanceRead, coords, rayleigh, mie);
imageStore(deltaRayleighScatteringWrite, texel, vec4(rayleigh, 1.0));
imageStore(deltaMieScatteringWrite, texel, vec4(mie, 1.0));
imageStore(scatteringWrite, texel, vec4(rayleigh, mie.r));
imageStore(singleMieScatteringWrite, texel, vec4(mie, 1.0));
""",
        "ComputeScatteringDensity": """
vec3 value = ComputeScatteringDensityTexture(ATMOSPHERE, transmittanceRead,
    singleRayleighScatteringRead, singleMieScatteringRead, multipleScatteringRead,
    irradianceRead, coords, scatteringOrder);
imageStore(deltaScatteringDensityWrite, texel, vec4(value, 1.0));
""",
        "ComputeIndirectIrradiance": """
vec3 value = ComputeIndirectIrradianceTexture(ATMOSPHERE,
    singleRayleighScatteringRead, singleMieScatteringRead, multipleScatteringRead,
    coords.xy, scatteringOrder);
imageStore(deltaIrradianceWrite, texel.xy, vec4(value, 1.0));
imageStore(irradianceWrite, texel.xy, textureLod(irradianceRead,
    coords.xy / vec2(IRRADIANCE_TEXTURE_WIDTH, IRRADIANCE_TEXTURE_HEIGHT), 0.0) + vec4(value, 1.0));
""",
        "ComputeMultipleScattering": """
float nu;
vec3 value = ComputeMultipleScatteringTexture(ATMOSPHERE, transmittanceRead,
    deltaScatteringDensityRead, coords, nu);
imageStore(deltaMultipleScatteringWrite, texel, vec4(value, 1.0));
vec4 accumulated = textureLod(scatteringRead,
    coords / vec3(SCATTERING_TEXTURE_MU_S_SIZE * SCATTERING_TEXTURE_NU_SIZE,
    SCATTERING_TEXTURE_MU_SIZE, SCATTERING_TEXTURE_R_SIZE), 0.0);
imageStore(scatteringWrite, texel, accumulated + vec4(value / RayleighPhaseFunction(nu), 0.0));
""",
    }


def compileKernels(build_directory, header):
    kernel_configs = {}
    kernel_bodies = makeKernels()
    for name, body in kernel_bodies.items():
        shader_file = build_directory / (name + ".comp")
        spirv_file = build_directory / (name + ".spv")
        metal_file = build_directory / (name + ".metal")
        invocation = """
void main() {
    ivec3 texel = ivec3(gl_GlobalInvocationID.xy, layer);
    vec3 coords = vec3(texel) + vec3(0.5);
"""
        shader_file.write_text(header + invocation + body + "}\n")
        runCommand(["glslangValidator", "-V", "-Os", str(shader_file), "-o", str(spirv_file)])
        result = subprocess.check_output(["spirv-cross", str(spirv_file), "--msl"])
        metal_file.write_bytes(result)
        reflection_data = subprocess.check_output(["spirv-cross", str(spirv_file), "--reflect"])
        reflection = json.loads(reflection_data)
        uniforms = reflection.get("ubos", [])
        uniform = uniforms[0] if uniforms else {"block_size": 0}
        members = reflection["types"][uniform["type"]]["members"] if uniforms else []
        metal = result.decode()
        texture_matches = re.findall(
            r"texture(?:2d|3d)<[^>]+> (\w+) \[\[texture\((\d+)\)\]\]", metal
        )
        textures = []
        for texture_name, texture_index in texture_matches:
            textures.append({"name": texture_name, "index": int(texture_index)})
        kernel_configs[name] = {
            "members": members,
            "size": uniform["block_size"],
            "textures": textures,
        }
    configuration = {
        "kernels": kernel_configs,
        "params": {
            "layer": 0,
            "scatteringOrder": 0,
            "model_bottom_radius": 6360.0,
            "model_top_radius": 6420.0,
        },
    }
    (build_directory / "config.json").write_text(json.dumps(configuration, indent=2))


def rsDataRows(float_values):
    """Hex rows of eight half-precision RGBA texels, one texel per sixteen characters."""
    texel_rows = []
    for index in range(0, len(float_values), 4):
        texel = float_values[index:index + 4]
        bits = [struct.unpack_from("<e", struct.pack("<f", value))[0] for value in texel]
        texel_rows.append("".join(format(word, "04x") for word in bits))
    return ["".join(texel_rows[row:row + 8]) for row in range(0, len(texel_rows), 8)]


def rsDataFile(tables):
    """Assemble sky-atmosphere-data.rs from the exported float tables."""
    constants = {"transmittance": "TRANSMITTANCE_HEX", "scattering": "SCATTERING_HEX", "irradiance": "IRRADIANCE_HEX"}
    blocks = []
    for name in ["transmittance", "scattering", "irradiance"]:
        rows = rsDataRows(tables[name])
        # One string literal spanning every data line, opened by the
        # declaration and closed on the final row.
        opening = f"pub(super) const {constants[name]}_HEX: &str = \"{rows[0]}"
        if len(rows) == 1:
            blocks.append(f"{opening}\";\n")
            continue
        lines = [opening]
        lines.extend(rows[1:-1])
        lines.append(f"{rows[-1]}\";")
        blocks.append("\n".join(lines) + "\n")
    header = """//! The four-order Bruneton precomputed atmosphere, as the editor's
//! viewport samples it, in the tables' packed order and dimensions: the
//! transmittance texture (256 x 64), the combined scattering texture
//! (256 x 128 x 32) and the indirect irradiance texture (64 x 16). One
//! texel is sixteen hexadecimal characters: red, green, blue and the
//! packed single-Mie channel, each a half-precision bit pattern, and a
//! line holds eight texels in the texture's row-major layout. Their
//! reproducible precomputation and the independent reference comparison
//! live in tools/sky-atmosphere; regeneration writes this file. The
//! sampler that reads them is sky-atmosphere.rs.
"""
    return header + "\n".join(blocks)


def packageTables(build_directory, output_file):
    tables = {}
    for name in ["transmittance", "scattering", "irradiance"]:
        float_values = array("f")
        float_values.frombytes((build_directory / (name + ".rgba32f")).read_bytes())
        assert len(float_values) == {"transmittance": 256 * 64, "scattering": 256 * 128 * 32, "irradiance": 64 * 16}[name] * 4, f"{name} has the declared texel count"
        tables[name] = float_values
    output_file.parent.mkdir(parents=True, exist_ok=True)
    data_file = rsDataFile(tables)
    output_file.write_text(data_file)
    print(f"Wrote {output_file}: {len(data_file):,} characters")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-directory", type=Path)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--no-package", action="store_true")
    arguments = parser.parse_args()
    for executable in ["glslangValidator", "spirv-cross", "swift"]:
        if shutil.which(executable) is None:
            parser.error(f"Missing prerequisite: {executable}")
    build_directory = arguments.build_directory
    if build_directory is None:
        build_directory = Path(tempfile.mkdtemp(prefix="nerthus-sky-atmosphere-"))
    build_directory.mkdir(parents=True, exist_ok=True)
    print(f"Building in {build_directory}", flush=True)
    definitions = fetchSource("definitions.glsl", build_directory)
    functions = fetchSource("functions.glsl", build_directory)
    functions = patchFunctions(functions)
    header = makeHeader(definitions, functions)
    compileKernels(build_directory, header)
    swift_script = Path(__file__).with_suffix(".swift")
    runCommand(["swift", str(swift_script), str(build_directory)])
    if not arguments.no_package:
        packageTables(build_directory, arguments.output)
    print(f"Canonical sky radiances: {build_directory / 'canonical-samples.json'}")


if __name__ == "__main__":
    main()
