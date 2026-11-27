// A small compute run for the shader tests of this crate and mono-look's.

use wgpu::util::DeviceExt;

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Gpu {
    /// A device of our own; none without an adapter, and the test says so.
    /// Under `GREYCARD_REQUIRE_GPU` a skip is a failure.
    pub fn new(what: &str) -> Option<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let got = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()
        .and_then(|adapter| {
            println!("{what} on {}", adapter.get_info().name);
            pollster::block_on(adapter.request_device(&Default::default())).ok()
        });
        if got.is_none() {
            assert!(
                std::env::var_os("GREYCARD_REQUIRE_GPU").is_none_or(|v| v.is_empty()),
                "{what} has no GPU to run on and GREYCARD_REQUIRE_GPU is set"
            );
            println!("SKIPPED: {what} has no GPU to run on");
        }
        got.map(|(device, queue)| Self { device, queue })
    }

    /// A pipeline of `source`, which declares its uniforms from binding 2
    /// of group 0 on, and a `fn eval(c: vec4<f32>) -> vec4<f32>`.
    pub fn pipeline(&self, source: &str) -> wgpu::ComputePipeline {
        let source = format!(
            "@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
             @group(0) @binding(1) var<storage, read_write> outs: array<vec4<f32>>;
             {source}
             @compute @workgroup_size(64)
             fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
                 if (id.x < arrayLength(&cases)) {{ outs[id.x] = eval(cases[id.x]); }}
             }}"
        );
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        self.device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
    }

    // Each test crate that includes this file calls one of the two.
    #[allow(dead_code)]
    pub fn run(
        &self,
        pipeline: &wgpu::ComputePipeline,
        uniforms: &[&[u8]],
        cases: &[[f32; 4]],
    ) -> Vec<[f32; 4]> {
        self.run_with(pipeline, uniforms, &[], cases)
    }

    /// [`Gpu::run`] with read only storage buffers bound after the uniforms.
    #[allow(dead_code)]
    pub fn run_with(
        &self,
        pipeline: &wgpu::ComputePipeline,
        uniforms: &[&[u8]],
        storage: &[&[u8]],
        cases: &[[f32; 4]],
    ) -> Vec<[f32; 4]> {
        let bytes: &[u8] = bytemuck::cast_slice(cases);
        let size = bytes.len() as u64;
        let filled = |contents: &[u8], usage| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents,
                    usage,
                })
        };
        let empty = |usage| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let mut buffers = vec![
            filled(bytes, wgpu::BufferUsages::STORAGE),
            empty(wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC),
        ];
        buffers.extend(
            uniforms
                .iter()
                .map(|u| filled(u, wgpu::BufferUsages::UNIFORM)),
        );
        buffers.extend(
            storage
                .iter()
                .map(|s| filled(s, wgpu::BufferUsages::STORAGE)),
        );
        let back = empty(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((cases.len() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&buffers[1], 0, &back, 0, size);
        self.queue.submit([encoder.finish()]);
        let slice = back.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("the results map"));
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device answers");
        let view = slice.get_mapped_range().expect("the results are mapped");
        bytemuck::cast_slice(&view).to_vec()
    }
}

/// splitmix64.
pub struct Rng(pub u64);

impl Rng {
    pub fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }

    /// The ends come up a fifth of the time each.
    pub fn slider(&mut self, lo: f32, hi: f32) -> f32 {
        (lo + (hi - lo) * (self.unit() * 1.6 - 0.3)).clamp(lo, hi)
    }
}

/// The largest difference of one channel of the results.
pub fn worst(gpu: &[[f32; 4]], channel: usize, cpu: impl Fn(usize) -> f32) -> f32 {
    gpu.iter()
        .enumerate()
        .map(|(i, g)| (g[channel] - cpu(i)).abs())
        .fold(
            0.0,
            |a, b| if b.is_nan() { f32::INFINITY } else { a.max(b) },
        )
}
