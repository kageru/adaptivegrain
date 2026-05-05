# Adaptivegrain-rs
Reimplementation of the adaptive\_grain mask as a Vapoursynth plugin.
For a description of the math and the general idea,
see [the article](https://blog.kageru.moe/legacy/adaptivegrain.html).

## Usage
```py
core.adg.Mask(clip, luma_scaling: float)
```

You must call `std.PlaneStats()` before this plugin
  (or fill the PlaneStatsAverage frame property using some other method).
Supported formats are YUV with 8-32 bit precision integer or single precision float.
Half precision float input is not supported since no one seems to be using that anyway.
Since the output is grey and only luma is processed,
  the subsampling of the input does not matter.

To replicate the original behaviour of adaptivegrain, a wrapper is provided in kagefunc.
The plugin behaves exactly like the original implementation
  (except for the performance, which is about 3x faster on my machine).

### Parameters
```
clip: vapoursynth.VideoNode
```
the input clip to generate a mask for.

```py
luma_scaling: float = 10.0
```
the luma\_scaling factor as described in the blog post.
Lower values will make the mask brighter overall.

## Build instructions
If you’re on Arch Linux,
  there’s an [AUR package](https://aur.archlinux.org/packages/vapoursynth-plugin-adaptivegrain-git/) for this plugin.
Otherwise you’ll have to build and install the package manually or use pip
```sh
cargo build --release
# or
pip install vapoursynth-adaptivegrain
```
Since version 0.4, binaries for Windows, Linux, and Mac are automatically published to pypi,
so they’re no longer uploaded to the Github release tab.
The Python packages include versions targetting different microarchitectures
according to the selection introduced in Vapoursynth R75.

## FAQ
**Why do I have to call std.PlaneStats() manually?**

~~Because I didn’t want to reimplement it. `kagefunc.adaptive_grain(clip, show_mask=True)` does that for you and then just returns the mask.~~
Because I was too dumb to realize [this](http://www.vapoursynth.com/doc/api/vapoursynth.h.html#invoke) exists.
I’ll fix that at some point.™

**Why doesn’t this also add grain?**

I was going to do that originally,
  but I didn’t want to reimplement grain
  when we already have a working grain filter
  (multiple even, which gives you the option to choose whichever you want).
