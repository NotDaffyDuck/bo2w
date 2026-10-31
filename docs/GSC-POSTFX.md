# Post effects driven by GSC

Script decides which post effects are shown. Presets and dvars reach the GPU
inside snapshot metadata, and the player's own brightness, depth-of-field,
bloom and debug-tweak settings never cancel an effect the script asked for.
Everything resets when the match ends.

## Vision, blur and focus

* `VisionSetNaked`, `VisionSetNight`, and the other `VisionSet...` calls, in
  their global form `(name, seconds)` and the per-player `self ...ForPlayer(name,
  seconds)` form, load `vision/<name>.vision`. An empty name returns to the map
  preset, and a global change wipes the player overrides on that channel. Night
  follows the night-vision flag of the player. Pain mixes into the base vision
  once health drops under half and fades back as it recovers.
* `self SetBlur(radius, seconds)` eases a Gaussian screen blur in; 0 removes it,
  and a new transition starts from the blur showing at that moment.
* `self SetDepthOfField(nearStart, nearEnd, farStart, farEnd, nearBlur, farBlur)`
  sets focus and blur. Zero distance ranges hand control back to autofocus.

## Dvars

`SetDvar(name, value)` is global and `self SetClientDvar(s)` overrides it for one
client.

* Film: enable, brightness, contrast, desaturate and dark desaturate, invert,
  and light/medium/dark RGB tints. Both `r_film...` and `r_filmTweak...` names work.
* Bloom: `r_glow`, radius0, bloom intensity0, cutoff and desaturation, also under
  the `r_glowTweak...` names.
* Depth of field: `r_dof_enable`, viewmodel/near/far start and end, near/far
  blur, bias.
* Extras beyond the film set: `r_hue` / `r_filmHue` (degrees), `r_gamma` (above
  0), `r_exposure` (stops), `r_saturation` (1 is neutral), `r_blur`,
  `r_brightness`, `r_contrast`. They are dvars, not new GSC natives.

## Fog and shake

* `SetExpFog(start, halfway, r, g, b, [opacity,] seconds)` changes the map fog,
  and the 14-argument sun-fog form is accepted. Transitions blend density and
  packed colour from the current state; the first call takes effect instantly.
* `Earthquake(scale, seconds, origin, radius)` adds a timed camera shake. Calls
  stack, weaken linearly with distance and fade out over their duration. A radius
  of 0 means global, and each call clamps its scale to 1 for camera angles. The
  shake is purely visual and leaves aim and collision alone.

```c
self SetClientDvars("r_filmBrightness", 0.1, "r_hue", 90, "r_gamma", 1.2);
self SetClientDvar("r_filmLightTint", (1, 0.5, 0.25));
self SetBlur(6, 0.5);
```

## Render order

The chain grades colour, blurs the scene, then applies film, depth of field and
bloom. The HUD stays legible, and bloom keeps the sRGB writes the material was
authored with. Snapshots use game protocol 95 (`net::PROTOCOL_VERSION`); host and
client must agree on it.
