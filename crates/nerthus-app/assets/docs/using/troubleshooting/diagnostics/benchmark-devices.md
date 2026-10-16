# Benchmark Devices

## Hardware Benchmark

The Hardware Benchmark runs a test to determine the general speed of your CPU, integrated GPU, dedicated GPU, and any other compatible devices that Nerthus can utilize for processing. Run it from Diagnostics (**Run Benchmark**) or from Options › Compute (**Scan Hardware and Determine Speeds**).

The score signifies the performance factor of each device. A higher score indicates better performance. The scores are kept in the data folder's `Benchmark.json`.

Diagnostics offers three accuracy settings: **Run Benchmark** uses 10 timed passes per device, **High Accuracy** uses 20, and **Very High Accuracy** uses 50. More passes take longer and reduce the effect of timing variation. Each score uses the average time per pass, so the different accuracy settings use the same scoring scale. Options › Compute uses the standard 10 passes.

### Compute Options

The benchmark data is also available via the [Compute](../../../ui/interface/options/compute.md) tab.
