//! Renderização do padrão de teste (wgpu/D3D12) para P1 (surface) e P2 (offscreen + readback).
use std::collections::HashMap;
use std::num::NonZeroIsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use serde_json::{Value, json};

pub const BLOCK_PX: u32 = 24;
pub const CODE_BLOCKS: u32 = 18;
pub const HEADER_BYTES: usize = 64;

pub struct Gpu {
    /// Mantido vivo enquanto o device/surface existirem.
    pub _instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Cria instance (DX12, como no ADR-033), a surface do HWND filho e o device compatível.
    pub fn new(child_hwnd: isize) -> Result<(Self, wgpu::Surface<'static>), String> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::DX12;
        let instance = wgpu::Instance::new(desc);
        let hwnd = NonZeroIsize::new(child_hwnd).ok_or("HWND nulo")?;
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
                raw_window_handle: RawWindowHandle::Win32(Win32WindowHandle::new(hwnd)),
            })
        }
        .map_err(|e| format!("create_surface_unsafe: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|e| format!("request_adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("s1-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .map_err(|e| format!("request_device: {e}"))?;
        Ok((
            Self {
                _instance: instance,
                adapter,
                device,
                queue,
            },
            surface,
        ))
    }

    pub fn describe(&self) -> Value {
        let i = self.adapter.get_info();
        json!({
            "name": i.name, "backend": format!("{:?}", i.backend), "device_type": format!("{:?}", i.device_type),
            "driver": i.driver, "driver_info": i.driver_info, "vendor": i.vendor, "device": i.device,
        })
    }
}

pub struct Pattern {
    layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    bind_group: wgpu::BindGroup,
    ubuf: wgpu::Buffer,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl Pattern {
    pub fn new(gpu: &Gpu) -> Self {
        let d = &gpu.device;
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("s1-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let ubuf = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("s1-uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("s1-bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            }],
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("s1-layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("s1-pattern"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pattern.wgsl").into()),
        });
        Self {
            layout,
            module,
            bind_group,
            ubuf,
            pipelines: HashMap::new(),
        }
    }

    fn pipeline(&mut self, gpu: &Gpu, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        self.pipelines.entry(format).or_insert_with(|| {
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("s1-pipeline"),
                    layout: Some(&self.layout),
                    vertex: wgpu::VertexState {
                        module: &self.module,
                        entry_point: Some("vs"),
                        buffers: &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.module,
                        entry_point: Some("fs"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
        })
    }

    #[allow(clippy::too_many_arguments)]
    /// Grava o uniform e codifica o passe que desenha o padrão em `view`.
    pub fn encode(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        frame: u32,
        size: (u32, u32),
        t: f32,
    ) {
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&frame.to_le_bytes());
        bytes[4..8].copy_from_slice(&(size.0 as f32).to_le_bytes());
        bytes[8..12].copy_from_slice(&(size.1 as f32).to_le_bytes());
        bytes[12..16].copy_from_slice(&t.to_le_bytes());
        gpu.queue.write_buffer(&self.ubuf, 0, &bytes);
        let bind_group = self.bind_group.clone();
        let pipeline = self.pipeline(gpu, format).clone();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("s1-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Anel (frame % 65536) -> instante de submissão em µs desde `base` (0 = nunca). Lido pelo sampler.
pub struct SubmitLog {
    pub base: Instant,
    slots: Vec<AtomicU64>,
}

impl SubmitLog {
    pub fn new(base: Instant) -> Arc<Self> {
        Arc::new(Self {
            base,
            slots: (0..65536).map(|_| AtomicU64::new(0)).collect(),
        })
    }
    pub fn record(&self, frame: u32) {
        let us = self.base.elapsed().as_micros() as u64 + 1;
        self.slots[(frame & 0xFFFF) as usize].store(us, Ordering::Release);
    }
    pub fn get(&self, frame: u16) -> Option<u64> {
        match self.slots[frame as usize].load(Ordering::Acquire) {
            0 => None,
            v => Some(v - 1),
        }
    }
    pub fn clear(&self) {
        for s in &self.slots {
            s.store(0, Ordering::Relaxed);
        }
    }
}

pub fn align256(n: u32) -> u32 {
    n.div_ceil(256) * 256
}

pub fn percentiles(values: &mut [f64]) -> Value {
    if values.is_empty() {
        return json!({ "count": 0 });
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    json!({ "count": values.len(), "mean": mean, "min": values[0], "p50": q(0.5), "p95": q(0.95), "p99": q(0.99), "max": values[values.len() - 1] })
}
