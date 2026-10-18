# Working with Satellite Maps

Our library of 2,026 color maps, taken from Copernicus Sentinel-2 satellite imagery, helps you colorize your terrains quickly without sacrificing realism.

The maps come from deserts, mountains, ice, forests, farmland, coasts and lakes all over the world, giving you a vast playground for colorizing your terrains.

A TextureMap node can be attached to a SatelliteMap node. Adapt the SatelliteMap to the mask as described in [Crafting Masks](crafting-masks.md).

## Editing Satellite Maps

There are a few ways to quickly tweak the Satellite Maps to better fit your needs.

### Map Bias

By moving the Bias slider, you can have the SatelliteMap apply more towards the left (negative values) or right (positive values) of the selected Color Map.

### Map Clipping

You can use the Clip slider to tell Satellite Maps to only use the selected segment of the entire color map.

For example, clipping out most of the left half changes which colors influence the terrain.

#### Roughness

The Roughness setting scatters the pixels of the color map to add the desired level of chaos and distortion.

#### HSL

The final tweaks come from Hue/Saturation/Luminosity editing.

***

Satellite Maps are one of the crucial aspects of colorizing your terrains. You can elevate your Satellite Maps by combining them with a Mixer as explained in [Layering Textures](layering-textures.md).

And finally, add [HueErosion](../../reference/nodes/colorize/hue-erosion.md) to add a new level of realism to your texture maps.
