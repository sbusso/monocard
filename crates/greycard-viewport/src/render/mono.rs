//! A mono frame on the GPU (docs/monocard): its planes as single channel
//! textures in place of the RGBA picture, and the fragment shader that
//! finishes them, held to `greycard_render::mono`. Everything else of the
//! viewport (the geometry, the masks, the overlays, the scopes, the
//! display table) is the stock shader's, which this one is joined to.

use std::sync::Weak;

use bytemuck::Zeroable;
use mono_core::{MonoEdit, MonoFrame, MonoLocal, Plane};
use mono_tone::point::{Compiled, LocalCompiled, MonoLocalParams, Paper};
use rayon::prelude::*;

use super::*;

/// The shader a mono frame is drawn with.
pub(super) fn shader() -> String {
    [
        include_str!("../viewport.wgsl"),
        mono_tone::point::WGSL,
        mono_look::WGSL,
        include_str!("../mono.wgsl"),
    ]
    .join("\n")
}

/// What one frame of the viewport shows of a mono edit, beside the stock
/// [`View`]'s geometry, masks, and marks. The stock look's fields do
/// nothing to a mono frame, the output sharpen among them, as the export
/// of one puts none on.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonoView {
    pub edit: MonoEdit,
    /// The masks' mono tools, one for each of [`View::locals`].
    pub locals: Vec<Option<MonoLocal>>,
    /// Paint the picture in this many zones.
    pub zone_view: Option<u32>,
    /// Hatch what the sensor clipped.
    pub clip_map: bool,
    /// Show it as it would look on this paper.
    pub paper: Option<Paper>,
    /// Draw the picture under this edit, with no mask, at the left of
    /// this fraction of the view.
    pub split: Option<(f32, Box<MonoEdit>)>,
}

impl MonoView {
    /// An edit's, with nothing painted over it.
    pub fn of(edit: &greycard_edit::Edit) -> Self {
        Self {
            edit: edit.mono.as_deref().cloned().unwrap_or_default(),
            locals: edit
                .adjustments
                .iter()
                .map(|a| a.mono.as_deref().cloned())
                .collect(),
            ..Self::default()
        }
    }

    /// With none of what is painted over the picture: what the scopes
    /// and the navigator are of.
    pub(super) fn plain(&self) -> Self {
        Self {
            zone_view: None,
            clip_map: false,
            split: None,
            ..self.clone()
        }
    }

    /// The edit as a histogram tapped at the curve reads it: nothing past
    /// the tap, and with `curves` false no curve, the locals' or the
    /// edit's. The Basic panel's sliders are before the curve and stay.
    pub(super) fn tapped(&self, curves: bool, locals: bool) -> Self {
        let mut v = self.plain();
        let rest = MonoEdit::default();
        v.paper = None;
        v.edit.look = rest.look;
        v.edit.display = rest.display;
        v.edit.grain = rest.grain;
        v.edit.vignette = rest.vignette;
        v.edit.solarize = rest.solarize;
        v.edit.toning = rest.toning;
        if !curves {
            v.edit.curve = rest.curve;
        }
        if !locals {
            for l in v.locals.iter_mut().flatten() {
                l.curve = Default::default();
            }
        }
        v
    }
}

/// The shader's `mono_view`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewParams {
    head: [f32; 4],
    planes: [f32; 4],
    locals: [u32; 4],
}

/// A plane on the GPU, and the one it holds, to know it again.
struct Held {
    texture: gpu::Texture,
    of: Option<Weak<Plane>>,
}

/// The planes in their binding order: the base, the guide, the detail
/// plane, the local contrast plane, the clip map. The base in half
/// floats, as the stock picture is: `the_mono_base_shows_no_bands` holds
/// it to the CPU's full ones.
const FORMATS: [gpu::TextureFormat; 5] = [
    gpu::TextureFormat::R16Float,
    gpu::TextureFormat::R16Float,
    gpu::TextureFormat::R16Float,
    gpu::TextureFormat::R16Float,
    gpu::TextureFormat::R8Unorm,
];
/// The clip map's place among them.
const CLIP: usize = 4;
/// The bindings after the planes: four uniforms.
const UNIFORMS: u32 = 4;

/// Rows sent at once, so that what is staged stays a few megabytes.
const BAND: usize = 256;

pub(super) struct Mono {
    layout: gpu::BindGroupLayout,
    pipeline: gpu::RenderPipeline,
    planes: [Held; 5],
    /// Which of the planes the frame has.
    has: [bool; 5],
    /// The frame's clip map, sent while it shows ([`Mono::sync_clip`]).
    clip: Option<Arc<Plane>>,
    iso: Option<f32>,
    white: f32,
    rendered: bool,
    /// The stock picture's binding, which this shader does not read.
    pub(super) none: gpu::Texture,
    /// A mono frame is the source.
    pub(super) on: bool,
}

/// A part of the view and what it is drawn with.
pub(super) struct Side {
    group: gpu::BindGroup,
    /// Its fractions across the view.
    from: f32,
    to: f32,
}

impl Mono {
    pub(super) fn new(device: &gpu::Device, stock: &gpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(gpu::ShaderModuleDescriptor {
            label: Some("mono viewport"),
            source: gpu::ShaderSource::Wgsl(shader().into()),
        });
        let entry = |binding: u32, ty| gpu::BindGroupLayoutEntry {
            binding,
            visibility: gpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let planes = (0..FORMATS.len()).map(|i| {
            entry(
                i as u32,
                gpu::BindingType::Texture {
                    sample_type: gpu::TextureSampleType::Float { filterable: true },
                    view_dimension: gpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            )
        });
        let uniforms = (0..UNIFORMS).map(|i| {
            entry(
                FORMATS.len() as u32 + i,
                gpu::BindingType::Buffer {
                    ty: gpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            )
        });
        let layout = device.create_bind_group_layout(&gpu::BindGroupLayoutDescriptor {
            label: Some("mono viewport"),
            entries: &planes.chain(uniforms).collect::<Vec<_>>(),
        });
        let pipeline_layout = device.create_pipeline_layout(&gpu::PipelineLayoutDescriptor {
            label: Some("mono viewport"),
            bind_group_layouts: &[Some(stock), Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&gpu::RenderPipelineDescriptor {
            label: Some("mono viewport"),
            layout: Some(&pipeline_layout),
            vertex: gpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(gpu::FragmentState {
                module: &module,
                entry_point: Some("fs_mono"),
                compilation_options: Default::default(),
                targets: &[Some(TARGET_FORMAT.into())],
            }),
            primitive: gpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: gpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            layout,
            planes: FORMATS.map(|f| Held {
                texture: texture(device, f, 1, 1),
                of: None,
            }),
            has: [false; 5],
            clip: None,
            iso: None,
            white: 1.0,
            rendered: false,
            none: texture(device, gpu::TextureFormat::Rgba16Float, 1, 1),
            on: false,
        }
    }

    /// The planes let go of: another picture is the source.
    pub(super) fn off(&mut self, device: &gpu::Device) {
        if std::mem::take(&mut self.on) {
            for (held, f) in self.planes.iter_mut().zip(FORMATS) {
                *held = Held {
                    texture: texture(device, f, 1, 1),
                    of: None,
                };
            }
            self.clip = None;
        }
    }

    /// `frame` on the GPU, but for the planes that are there already.
    fn put(&mut self, device: &gpu::Device, queue: &gpu::Queue, frame: &MonoFrame, most: u32) {
        let planes = [
            Some(&frame.base),
            frame.guide.as_ref(),
            frame.detail.as_ref(),
            frame.local.as_ref(),
        ];
        for (i, plane) in planes.into_iter().enumerate() {
            self.put_one(device, queue, i, plane, most);
        }
        self.clip = frame.clip.clone();
        self.iso = frame.iso;
        self.white = frame.white;
        self.rendered = frame.rendered;
        self.on = true;
    }

    /// Plane `i`, or its absence, but for one that is there already.
    fn put_one(
        &mut self,
        device: &gpu::Device,
        queue: &gpu::Queue,
        i: usize,
        plane: Option<&Arc<Plane>>,
        most: u32,
    ) {
        let held = &mut self.planes[i];
        self.has[i] = plane.is_some();
        let Some(plane) = plane else {
            if held.of.take().is_some() {
                held.texture = texture(device, FORMATS[i], 1, 1);
            }
            return;
        };
        let same = held.of.as_ref().and_then(Weak::upgrade);
        if !same.is_some_and(|h| Arc::ptr_eq(&h, plane)) {
            upload(device, queue, &mut held.texture, plane, most);
            held.of = Some(Arc::downgrade(plane));
        }
    }

    /// The clip map on the GPU while it shows, and off it while not.
    fn sync_clip(&mut self, device: &gpu::Device, queue: &gpu::Queue, shown: bool, most: u32) {
        let clip = self.clip.clone().filter(|_| shown && self.on);
        self.put_one(device, queue, CLIP, clip.as_ref(), most);
    }

    pub(super) fn base(&self) -> &gpu::Texture {
        &self.planes[0].texture
    }

    /// What the view of `v` is drawn with, part by part; the paper on the
    /// way to the monitor alone.
    pub(super) fn sides(
        &self,
        device: &gpu::Device,
        queue: &gpu::Queue,
        v: &View,
        monitor: bool,
    ) -> Vec<Side> {
        let at_rest = MonoView::default();
        let m = v.mono.as_ref().unwrap_or(&at_rest);
        let after = self.group(device, queue, m, &m.edit, &m.locals, monitor);
        let side = |group, from, to| Side { group, from, to };
        match &m.split {
            Some((at, before)) if monitor => {
                let at = at.clamp(0.0, 1.0);
                let left = self.group(device, queue, m, before, &[], monitor);
                vec![side(left, 0.0, at), side(after, at, 1.0)]
            }
            _ => vec![side(after, 0.0, 1.0)],
        }
    }

    fn group(
        &self,
        device: &gpu::Device,
        queue: &gpu::Queue,
        m: &MonoView,
        edit: &MonoEdit,
        locals: &[Option<MonoLocal>],
        monitor: bool,
    ) -> gpu::BindGroup {
        let ev = edit.basic.exposure;
        let tone = Compiled::of(edit, self.white * ev.exp2(), self.rendered).params();
        let look = mono_look::Compiled::new(edit, self.iso).params();
        let mut masks = [MonoLocalParams::zeroed(); MAX_LOCALS];
        let mut on = 0u32;
        let (mut zones, mut local_contrast) = (!edit.tone_eq.is_neutral(), false);
        for (k, l) in locals.iter().enumerate().take(MAX_LOCALS) {
            let Some(l) = l else { continue };
            masks[k] = LocalCompiled::with_tone_eq(l, &edit.tone_eq).params();
            on |= 1 << k;
            zones |= l.zones.iter().any(|z| *z != 0.0);
            local_contrast |= l.local_contrast != 0.0;
        }
        let flag = |b: bool| f32::from(u8::from(b));
        let view = ViewParams {
            head: [
                ev,
                m.zone_view.map_or(0.0, |n| n.max(2) as f32),
                0.0,
                m.paper
                    .filter(|_| monitor)
                    .and_then(|p| Paper::ALL.iter().position(|a| *a == p))
                    .map_or(0.0, |i| (i + 1) as f32),
            ],
            planes: [
                flag(self.has[1] && zones),
                flag(self.has[2] && edit.wants_detail()),
                flag(self.has[3] && local_contrast),
                flag(self.has[CLIP] && m.clip_map),
            ],
            locals: [on, 0, 0, 0],
        };
        let uniform = |bytes: &[u8]| {
            let buffer = device.create_buffer(&gpu::BufferDescriptor {
                label: Some("mono params"),
                size: bytes.len() as u64,
                usage: gpu::BufferUsages::UNIFORM | gpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytes);
            buffer
        };
        let buffers: [_; UNIFORMS as usize] = [
            uniform(bytemuck::bytes_of(&tone)),
            uniform(bytemuck::cast_slice(&masks)),
            uniform(bytemuck::bytes_of(&look)),
            uniform(bytemuck::bytes_of(&view)),
        ];
        let views = self
            .planes
            .each_ref()
            .map(|h| h.texture.create_view(&Default::default()));
        let entries: Vec<_> = views
            .iter()
            .map(gpu::BindingResource::TextureView)
            .chain(buffers.iter().map(|b| b.as_entire_binding()))
            .enumerate()
            .map(|(i, resource)| gpu::BindGroupEntry {
                binding: i as u32,
                resource,
            })
            .collect();
        device.create_bind_group(&gpu::BindGroupDescriptor {
            label: Some("mono viewport"),
            layout: &self.layout,
            entries: &entries,
        })
    }

    /// The draw itself, the stock group bound, into a target of `size`.
    pub(super) fn draw(&self, pass: &mut gpu::RenderPass<'_>, sides: &[Side], size: (u32, u32)) {
        pass.set_pipeline(&self.pipeline);
        let edge = |at: f32| ((size.0 as f32 * at).round() as u32).min(size.0);
        for side in sides {
            pass.set_bind_group(1, &side.group, &[]);
            // One side is the whole, whatever scissor is set.
            if sides.len() > 1 {
                let (from, to) = (edge(side.from), edge(side.to));
                if to <= from {
                    continue;
                }
                pass.set_scissor_rect(from, 0, to - from, size.1);
            }
            pass.draw(0..3, 0..1);
        }
    }

    #[cfg(test)]
    pub(super) fn bytes(&self) -> u64 {
        self.planes
            .iter()
            .map(|h| super::mono_tests::bytes(&h.texture))
            .sum()
    }
}

fn texture(
    device: &gpu::Device,
    format: gpu::TextureFormat,
    width: u32,
    height: u32,
) -> gpu::Texture {
    device.create_texture(&gpu::TextureDescriptor {
        label: Some("mono plane"),
        size: gpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: gpu::TextureDimension::D2,
        format,
        usage: gpu::TextureUsages::TEXTURE_BINDING
            | gpu::TextureUsages::COPY_DST
            | gpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// `plane` into `into`, made anew where it is another size, reduced to
/// fit the device as the stock picture is ([`reduce`]), a band of rows at
/// a time.
fn upload(
    device: &gpu::Device,
    queue: &gpu::Queue,
    into: &mut gpu::Texture,
    plane: &Plane,
    most: u32,
) {
    let (w, h) = (plane.width as u32, plane.height as u32);
    let k = reduce::factor(w, h, most);
    let reduced;
    let (w, h, data) = if k > 1 {
        reduced = reduce::plane::<f32, 1>(
            &plane.data,
            plane.width,
            plane.height,
            k as usize,
            |v| v,
            |v| v,
        );
        let (w, h) = reduce::reduced_size(w, h, k);
        (w, h, &reduced)
    } else {
        (w, h, &plane.data)
    };
    let format = into.format();
    if (into.width(), into.height()) != (w, h) {
        *into = texture(device, format, w, h);
    }
    for (i, band) in data.chunks(w as usize * BAND).enumerate() {
        let bytes: Vec<u8> = match format {
            gpu::TextureFormat::R16Float => {
                let most = half::f16::MAX.to_f32();
                let halves: Vec<half::f16> = band
                    .par_iter()
                    .map(|v| half::f16::from_f32(v.clamp(-most, most)))
                    .collect();
                bytemuck::cast_slice(&halves).to_vec()
            }
            _ => band
                .par_iter()
                .map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
                .collect(),
        };
        let rows = (band.len() / w as usize) as u32;
        queue.write_texture(
            gpu::TexelCopyTextureInfo {
                texture: into,
                mip_level: 0,
                origin: gpu::Origin3d {
                    x: 0,
                    y: (i * BAND) as u32,
                    z: 0,
                },
                aspect: gpu::TextureAspect::All,
            },
            &bytes,
            gpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes.len() as u32 / rows),
                rows_per_image: Some(rows),
            },
            gpu::Extent3d {
                width: w,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        // Sent now, so that the staged band is freed once it is copied.
        queue.submit(None);
    }
}

impl Renderer {
    /// Put a mono develop on the GPU as the source, in the RGBA
    /// picture's place. Planes the last frame held, the same
    /// allocations, are not sent again, so a new detail plane costs that
    /// plane alone. One past the device's textures goes up reduced.
    pub fn set_mono(&mut self, frame: &MonoFrame) {
        let mono = self
            .mono
            .get_or_insert_with(|| Mono::new(&self.device, &self.layout));
        mono.put(&self.device, &self.queue, frame, self.most_side);
        self.source = Some(mono.base().clone());
        self.full = (frame.base.width as u32, frame.base.height as u32);
        self.encoded = false;
        self.scopes.pixels = None;
        self.scopes.new_source();
        self.redraw = true;
    }

    /// Whether the source is a mono frame.
    pub fn is_mono(&self) -> bool {
        self.mono.as_ref().is_some_and(|m| m.on)
    }

    /// What of the mono frame `v` shows and no other view does.
    pub(super) fn sync_mono(&mut self, v: &View) {
        if let Some(m) = self.mono.as_mut() {
            let shown = v.mono.as_ref().is_some_and(|m| m.clip_map);
            m.sync_clip(&self.device, &self.queue, shown, self.most_side);
        }
    }

    /// Another picture is the source: the mono planes go.
    pub(super) fn mono_off(&mut self) {
        if let Some(m) = self.mono.as_mut() {
            m.off(&self.device);
        }
    }
}
