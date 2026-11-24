# Open questions

## Answered by the code

- **Does Greycard render through wgpu, or through Vulkan and Metal directly?**
  Through wgpu. `greycard-viewport` draws with it and `greycard-gpu` runs its compute
  shaders with it, all in WGSL, so one shader serves every platform.
- **Does the denoiser's training code ship, or only the weights?** The training code
  ships, in `tools/denoise`. The weights are not in the repository: the app downloads
  them.

## Open

- What is the Ricoh GR IV Monochrome's top ISO, and does its DNG carry lens opcodes?
  Needs a file from the camera. The bundled profile's match strings need it too.
- Is Greycard's author open to a `mono` feature upstream? Ask before a public launch.
- What is the final name, after a trademark and domain check? The bundle id, the site
  address, and the repository wait on it.
- Is this a personal tool, a free release, or paid builds? The GPL allows selling
  builds. The source must stay open.
- Where are the calibration frames and the 40 golden DNGs kept? Nothing binary goes
  in the repository.
