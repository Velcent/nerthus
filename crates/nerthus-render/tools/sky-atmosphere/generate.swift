import Foundation
import Metal

guard CommandLine.arguments.count == 2 else {
    fatalError("Usage: generate.swift BUILD_DIRECTORY")
}
let directory = URL(fileURLWithPath: CommandLine.arguments[1])
let config_data = try Data(contentsOf: directory.appendingPathComponent("config.json"))
let config = try JSONSerialization.jsonObject(with: config_data) as! [String: Any]
let kernel_configs = config["kernels"] as! [String: [String: Any]]
var parameters = config["params"] as! [String: Any]
guard let device = MTLCreateSystemDefaultDevice(),
      let queue = device.makeCommandQueue() else {
    fatalError("Metal unavailable")
}
print("Device \(device.name)")

var pipelines: [String: MTLComputePipelineState] = [:]
for name in kernel_configs.keys.sorted() {
    let source = try String(contentsOf: directory.appendingPathComponent(name + ".metal"), encoding: .utf8)
    let options = MTLCompileOptions()
    options.mathMode = .fast
    let library = try device.makeLibrary(source: source, options: options)
    let function = library.makeFunction(name: "main0")!
    pipelines[name] = try device.makeComputePipelineState(function: function)
    print("Compiled \(name)")
}

let sampler_descriptor = MTLSamplerDescriptor()
sampler_descriptor.minFilter = .linear
sampler_descriptor.magFilter = .linear
sampler_descriptor.mipFilter = .notMipmapped
sampler_descriptor.sAddressMode = .clampToEdge
sampler_descriptor.tAddressMode = .clampToEdge
sampler_descriptor.rAddressMode = .clampToEdge
let sampler = device.makeSamplerState(descriptor: sampler_descriptor)!

func makeTexture(_ name: String, _ width: Int, _ height: Int, _ depth: Int, _ pixel_format: MTLPixelFormat = .rgba16Float) -> MTLTexture {
    let descriptor = MTLTextureDescriptor()
    descriptor.textureType = depth > 1 ? .type3D : .type2D
    descriptor.pixelFormat = pixel_format
    descriptor.width = width
    descriptor.height = height
    descriptor.depth = depth
    descriptor.storageMode = .shared
    descriptor.usage = [.shaderRead, .shaderWrite]
    let texture = device.makeTexture(descriptor: descriptor)!
    texture.label = name
    let bytes_per_pixel = pixel_format == .rgba32Float ? 16 : 8
    let bytes_per_row = width * bytes_per_pixel
    let bytes_per_image = bytes_per_row * height
    let zero_data = Data(count: bytes_per_image * depth)
    zero_data.withUnsafeBytes { bytes in
        let region = MTLRegionMake3D(0, 0, 0, width, height, depth)
        texture.replace(region: region, mipmapLevel: 0, slice: 0,
                        withBytes: bytes.baseAddress!, bytesPerRow: bytes_per_row,
                        bytesPerImage: bytes_per_image)
    }
    return texture
}

let transmittance = makeTexture("transmittance",256,64,1)
let delta_irradiance = makeTexture("delta-irradiance",64,16,1)
var irradiance = [makeTexture("irradiance-a",64,16,1),makeTexture("irradiance-b",64,16,1)]
var scattering = [makeTexture("scattering-a",256,128,32),makeTexture("scattering-b",256,128,32)]
var single_mie = [makeTexture("single-mie-a",256,128,32),makeTexture("single-mie-b",256,128,32)]
let delta_rayleigh = makeTexture("delta-rayleigh",256,128,32)
let delta_mie = makeTexture("delta-mie",256,128,32)
let density = makeTexture("density",256,128,32)
let delta_multiple = delta_rayleigh

func dispatch(_ name: String, _ texture_map: [String: MTLTexture], _ width: Int, _ height: Int, _ layers: Int) {
    let kernel_config = kernel_configs[name]!
    let members = kernel_config["members"] as! [[String: Any]]
    let block_size = kernel_config["size"] as! Int
    let textures = kernel_config["textures"] as! [[String: Any]]
    let command = queue.makeCommandBuffer()!
    let encoder = command.makeComputeCommandEncoder()!
    encoder.setComputePipelineState(pipelines[name]!)
    for sampler_index in 0..<16 {
        encoder.setSamplerState(sampler, index: sampler_index)
    }
    for binding in textures {
        let texture_name = binding["name"] as! String
        guard let texture = texture_map[texture_name] else {
            fatalError("Missing \(name) \(texture_name)")
        }
        encoder.setTexture(texture, index: binding["index"] as! Int)
    }
    for layer in 0..<layers {
        parameters["layer"] = layer
        var block = Data(count: max(block_size, 512))
        for member in members {
            let name = member["name"] as! String
            let offset = member["offset"] as! Int
            let type = member["type"] as! String
            guard let value = parameters[name] else {
                fatalError("Missing parameter \(name)")
            }
            if type == "int" {
                var integer = Int32((value as! NSNumber).intValue)
                withUnsafeBytes(of: &integer) { bytes in block.replaceSubrange(offset..<(offset+4), with: bytes) }
            } else {
                let values = (value as? [NSNumber]) ?? [value as! NSNumber]
                for (index, number) in values.enumerated() {
                    var float = number.floatValue
                    let element_offset = offset + index * 4
                    withUnsafeBytes(of: &float) { bytes in block.replaceSubrange(element_offset..<(element_offset+4), with: bytes) }
                }
            }
        }
        block.withUnsafeBytes { bytes in encoder.setBytes(bytes.baseAddress!, length: block.count, index: 0) }
        let threadgroups = MTLSize(width: width / 8, height: height / 8, depth: 1)
        let threads = MTLSize(width: 8, height: 8, depth: 1)
        encoder.dispatchThreadgroups(threadgroups, threadsPerThreadgroup: threads)
    }
    encoder.endEncoding()
    command.commit()
    command.waitUntilCompleted()
    if let error = command.error { fatalError("\(name): \(error)") }
    print("Finished \(name) \(parameters["scatteringOrder"]!)")
}

func exportTexture(_ texture: MTLTexture, _ name: String) throws {
    let count = texture.width * texture.height * texture.depth * 4
    var raw = [UInt16](repeating: 0, count: count)
    raw.withUnsafeMutableBytes { bytes in
        let region = MTLRegionMake3D(0, 0, 0, texture.width, texture.height, texture.depth)
        texture.getBytes(bytes.baseAddress!, bytesPerRow: texture.width * 8,
                         bytesPerImage: texture.width * texture.height * 8,
                         from: region, mipmapLevel: 0, slice: 0)
    }
    let floats = raw.map { Float(Float16(bitPattern: $0)) }
    let data = floats.withUnsafeBytes { Data($0) }
    try data.write(to: directory.appendingPathComponent(name + ".rgba32f"))
    print("Exported \(name) \(data.count) bytes; max \(floats.filter { $0.isFinite }.max() ?? 0), nonfinite \(floats.filter { !$0.isFinite }.count)")
}

let transmittance_textures = ["transmittanceWrite": transmittance]
dispatch("ComputeTransmittance", transmittance_textures, 256, 64, 1)

let direct_irradiance_textures = [
    "transmittanceRead": transmittance,
    "deltaIrradianceWrite": delta_irradiance,
    "irradianceWrite": irradiance[1],
    "irradianceRead": irradiance[0],
]
dispatch("ComputeDirectIrradiance", direct_irradiance_textures, 64, 16, 1)
irradiance.swapAt(0, 1)

let single_scattering_textures = [
    "transmittanceRead": transmittance,
    "deltaRayleighScatteringWrite": delta_rayleigh,
    "deltaMieScatteringWrite": delta_mie,
    "scatteringWrite": scattering[1],
    "scatteringRead": scattering[0],
    "singleMieScatteringWrite": single_mie[1],
    "singleMieScatteringRead": single_mie[0],
]
dispatch("ComputeSingleScattering", single_scattering_textures, 256, 128, 32)
scattering.swapAt(0, 1)
single_mie.swapAt(0, 1)
try exportTexture(scattering[0], "scattering-order-1")

for order in 2...4 {
    parameters["scatteringOrder"] = order
    let density_textures = [
        "singleRayleighScatteringRead": delta_rayleigh,
        "singleMieScatteringRead": delta_mie,
        "multipleScatteringRead": delta_multiple,
        "deltaScatteringDensityWrite": density,
    ]
    dispatch("ComputeScatteringDensity", density_textures, 256, 128, 32)

    parameters["scatteringOrder"] = order - 1
    let indirect_irradiance_textures = [
        "singleRayleighScatteringRead": delta_rayleigh,
        "singleMieScatteringRead": delta_mie,
        "multipleScatteringRead": delta_multiple,
        "deltaIrradianceWrite": delta_irradiance,
        "irradianceWrite": irradiance[1],
        "irradianceRead": irradiance[0],
    ]
    dispatch("ComputeIndirectIrradiance", indirect_irradiance_textures, 64, 16, 1)
    irradiance.swapAt(0, 1)

    let multiple_scattering_textures = [
        "transmittanceRead": transmittance,
        "deltaScatteringDensityRead": density,
        "deltaMultipleScatteringWrite": delta_multiple,
        "scatteringWrite": scattering[1],
        "scatteringRead": scattering[0],
    ]
    dispatch("ComputeMultipleScattering", multiple_scattering_textures, 256, 128, 32)
    scattering.swapAt(0, 1)
    try exportTexture(scattering[0], "scattering-order-\(order)")
}

try exportTexture(transmittance, "transmittance")
try exportTexture(scattering[0], "scattering")
try exportTexture(irradiance[0], "irradiance")
try exportTexture(single_mie[0], "single-mie")

let samples = makeTexture("samples", 8, 8, 1, .rgba32Float)
let sample_textures = [
    "transmittanceRead": transmittance,
    "scatteringRead": scattering[0],
    "singleMieScatteringRead": single_mie[0],
    "sampleWrite": samples,
]
dispatch("ComputeSkySamples", sample_textures, 8, 8, 1)
var sample_values = [Float](repeating: 0, count: 8 * 8 * 4)
sample_values.withUnsafeMutableBytes { bytes in
    let region = MTLRegionMake2D(0, 0, 8, 8)
    samples.getBytes(bytes.baseAddress!, bytesPerRow: 8 * 16,
                     from: region, mipmapLevel: 0)
}
var canonical_samples: [[String: [Float]]] = []
for index in 0..<5 {
    let radiance_offset = index * 4
    let transmittance_offset = (index + 8) * 4
    let radiance = Array(sample_values[radiance_offset..<(radiance_offset + 3)])
    let transmission = Array(sample_values[transmittance_offset..<(transmittance_offset + 3)])
    let sample = ["radiance": radiance, "transmittance": transmission]
    canonical_samples.append(sample)
}
let sample_data = try JSONSerialization.data(withJSONObject: canonical_samples, options: [.prettyPrinted, .sortedKeys])
try sample_data.write(to: directory.appendingPathComponent("canonical-samples.json"))
print(String(data: sample_data, encoding: .utf8)!)
