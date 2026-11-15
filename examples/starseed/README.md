# Starseed

An original WebGPU game packed in one ODI file and run by the browser runner
(`web/`): a seed of light drifting through a field of motes. Gather the
gold ones, dodge the dark ones; three stages, each with its own music.

- `core/`: the game itself, Rust compiled to WebAssembly (no bindings: plain
  exported functions). It writes one instance (8 floats) per glowing thing.
- `site/`: the page. `game.js` draws those instances with WebGPU
  (`shaders/sprite.wgsl`, `shaders/sky.wgsl`) and plays sound. Each stage's
  music is fetched when the stage begins, so the runner reads it from the
  ODI file only then.
- `gen_audio.py`: makes the music (no samples).

```sh
examples/starseed/build.sh            # -> examples/starseed/starseed.odi
odi web examples/starseed/starseed.odi
```

Open the printed URL in a browser with WebGPU (Chrome, Edge, Safari 26+),
press "Save for offline", and it plays with no network from then on. Or
open the runner with no file and drop `starseed.odi` on it.
