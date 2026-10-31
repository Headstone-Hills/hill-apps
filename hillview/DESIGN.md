# HillView design notes

## 1. Host compatibility

**Scope update (Oct 2026):** Avid is dropped. DaVinci Resolve (free) is *assumed* to load third-party OpenFX plugins;
this is unverified and should be checked with the first test plugin. A standalone app (see README) ships alongside the plugins.

There is no single plugin format that loads in all four hosts, so the plan is one portable core
plus a thin adapter per host.

| Host | Plugin API | Evidence | Confidence |
|---|---|---|---|
| DaVinci Resolve (free) | OpenFX (OFX) | Sources agree free Resolve loads third-party OFX ([TryUncle, updated Jul 2026](https://tryuncle.com/learn/davinci-resolve/best-davinci-resolve-plugins-for-beginners-to-buy), [CGChannel 2018](https://www.cgchannel.com/?p=98909)); one outlier ([Tella](https://www.tella.com/definition/ofx-plugins)) says Studio-only. | Medium: **verify on Blackmagic's feature-comparison page and by loading a test OFX in free Resolve** |
| Premiere Pro | After Effects SDK effect (`.aex`) plus Premiere extensions | Adobe community: effect samples are built on the AE API with Premiere-specific extensions for transitions/GPU; CPU rendering from the AE SDK works ([thread](https://community.adobe.com/t5/premiere-pro-discussions/can-you-build-premiere-pro-plugins-using-the-after-effects-sdk/m-p/9100157)); Premiere refuses some AE flags ([2007 thread](https://community.adobe.com/t5/after-effects-discussions/how-to-install-aex-in-premiere-pro-2-0/m-p/1673953)). That Premiere does not host OFX is from my own knowledge, not a source. | Medium: sources are old |
| Final Cut Pro | FxPlug, delivered through Motion | Apple: [FxPlug](https://developer.apple.com/documentation/professional-video-applications/fxplug.md) is for effects FCP/Motion can't do natively; plug-ins reach FCP via a published Motion effect ([Apple guide](https://developer.apple.com/library/archive/documentation/AppleApplications/Conceptual/FXPlug_overview/FxPlugsFCPx/FxPlugsFCPx.html), [FxFactory](https://fxfactory.com/developer/bringing-native-plugins-to-finalcutpro/)). The FxFactory article is old; **check current FxPlug 4 docs**. macOS only. | Medium |

Order of work: Resolve (OFX) first, then Premiere (AE SDK), then Final Cut Pro (FxPlug, macOS/Metal, delivered via Motion).

## 2. Architecture

```
core/ (C++17, no host headers)     <- port of the Python reference
   frame ops + modulation + params + audio-envelope reader
adapters/
   ofx/      Resolve (and any OFX host: Nuke, Fusion, Vegas...)
   ae_sdk/   Premiere (and After Effects, nearly free)
   fxplug/   Final Cut Pro / Motion (Metal)
```

The Python engine is the specification: C++ output is checked against it frame-for-frame.
Two properties make porting tractable:
- **Stateless frames** (see README). Verified by a test.
- **All modulation is a pure function of time + seed**, so a host re-rendering frame 812 matches playback.

Temporal effects need neighbouring frames: sampling/interpolation fetches frames `k0` and `k0+N`
(OFX, AE and FxPlug all allow fetching other times; AVX2's ability here is **unverified**).
Each processed keyframe is cached; GPU versions (Metal/CUDA/OpenCL) come after CPU parity.

## 3. Audio-reactive modulation

Plugin APIs generally give effects video only, not the timeline audio. That is my understanding
rather than something I confirmed per-SDK, so treat it as an assumption to verify. The design does not
depend on it: `hillview analyze` turns an audio/video file into a host-neutral JSON of per-video-frame
envelopes (`rms, low, mid, high, onset`); the plugin loads the file via a path parameter. Same
file works in every host. Cost: re-run analysis if the edit changes the audio timing.

## 4. Decisions to revisit

- `blend` interpolation is a cross-fade, not motion-compensated; optical-flow interpolation is the
  obvious upgrade but a large cost on the host side.
- Variable `sampling.step` is evaluated at the output frame's time, so changing it mid-clip creates
  deliberate discontinuities rather than smooth ramps.
- k-means is seeded and therefore stable between frames; per-frame re-seeding as a "flicker"
  mode is not yet exposed.
- Box-blur Gaussian approximation is integer-radius, so very small sigma changes can step.
