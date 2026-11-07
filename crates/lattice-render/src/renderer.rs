//! GPU side: owns the device, surface, pipelines and per-section buffers.

use crate::atlas::{Atlas, MIP_LEVELS, Sheet};
use crate::mesher::{SectionMesh, Vertex};
use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu::{self, util::DeviceExt};
use glam::{DVec3, Mat4, Vec3, Vec4};
use std::collections::HashMap;
use std::path::PathBuf;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    sun: [f32; 4],
    fog: [f32; 4],
}

/// Vertex for outlines and simple coloured geometry (camera-relative).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ColorVertex {
    pub pos: [f32; 3],
    pub color: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub pos: DVec3,
    /// Degrees, game convention: 0 faces +Z (south), 90 faces −X (west).
    pub yaw: f32,
    /// Degrees, positive looks down.
    pub pitch: f32,
    pub fov_y_deg: f32,
    /// Rolled around the view axis, degrees (hurt tilt, walking).
    pub roll: f32,
    /// Extra look-down from walking, degrees.
    pub nod: f32,
    /// Moved in view space (walking).
    pub shift: Vec3,
    /// Warped by a portal or nausea: which way, in degrees, and how squeezed.
    pub warp: Option<(f32, f32)>,
}

impl Camera {
    pub fn forward(&self) -> Vec3 {
        let (y, p) = (self.yaw.to_radians(), self.pitch.to_radians());
        Vec3::new(-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos())
    }

    /// Turns the world to face this way. Built from the angles, not from a "look at"
    /// with the world's up: straight up or down those two line up and the sideways
    /// direction comes from rounding error, which turned the whole view around.
    pub fn turn(&self) -> Mat4 {
        Mat4::from_rotation_x(self.pitch.to_radians()) * Mat4::from_rotation_y(self.yaw.to_radians() + std::f32::consts::PI)
    }
}

/// Quads rebuilt every frame, in camera-relative space. Each list is four vertices per quad.
#[derive(Default)]
pub struct Batches {
    /// Cut-out, textured from the entity sheet.
    pub entity: Vec<Vertex>,
    pub entity_translucent: Vec<Vertex>,
    /// Added to what's behind it, unlit: eyes, energy.
    pub entity_glow: Vec<Vertex>,
    /// Blended on top of what's there, without marking depth: shadows on the ground.
    pub shadow: Vec<Vertex>,
    /// Shows through everything: the faint part of a name tag behind a wall.
    pub through: Vec<Vertex>,
    /// Sun, moon and stars: on the sky, added to it, unaffected by fog.
    pub celestial: Vec<Vertex>,
    pub clouds: Vec<Vertex>,
    /// Rain and snow, from their own repeating pictures.
    pub rain: Vec<Vertex>,
    pub snow: Vec<Vertex>,
    /// Textured from the block atlas: items, moving blocks.
    pub blocks: Vec<Vertex>,
    pub blocks_translucent: Vec<Vertex>,
    /// Drawn last, over everything, in view space with their own field of view.
    pub hand_entity: Vec<Vertex>,
    pub hand_blocks: Vec<Vertex>,
    /// What's over the whole view: being underwater or on fire, from the sheet and the atlas.
    pub overlay: Vec<Vertex>,
    pub overlay_blocks: Vec<Vertex>,
    /// The shine on enchanted things: on items, on worn armour, on the held item.
    pub glint_item: Vec<Vertex>,
    pub glint_armor: Vec<Vertex>,
    pub hand_glint: Vec<Vertex>,
    pub end_portal: Vec<Vertex>,
    pub energy_creeper: Vec<Vertex>,
    pub energy_wither: Vec<Vertex>,
}

impl Batches {
    pub fn clear(&mut self) {
        self.entity.clear();
        self.entity_translucent.clear();
        self.entity_glow.clear();
        self.shadow.clear();
        self.through.clear();
        self.celestial.clear();
        self.clouds.clear();
        self.rain.clear();
        self.snow.clear();
        self.blocks.clear();
        self.blocks_translucent.clear();
        self.hand_entity.clear();
        self.hand_blocks.clear();
        self.overlay.clear();
        self.overlay_blocks.clear();
        self.glint_item.clear();
        self.glint_armor.clear();
        self.hand_glint.clear();
        self.end_portal.clear();
        self.energy_creeper.clear();
        self.energy_wither.clear();
    }
}

/// Everything about the world that a frame needs.
pub struct FrameWorld<'a> {
    pub camera: &'a Camera,
    /// Linear RGB of the sky overhead and at the horizon (also the fog colour).
    pub sky_zenith: [f32; 3],
    pub sky_horizon: [f32; 3],
    pub sun_dir: Vec3,
    pub daylight: f32,
    pub fog_start: f32,
    pub fog_end: f32,
    pub min_brightness: f32,
    /// 0 moody to 1 bright.
    pub gamma: f32,
    pub lines: &'a [ColorVertex],
    pub triangles: &'a [ColorVertex],
    pub batches: &'a Batches,
    /// Field of view of the held item, degrees.
    pub hand_fov: f32,
}

pub struct EguiFrame<'a> {
    pub primitives: &'a [egui::ClippedPrimitive],
    pub textures: &'a mut egui::TexturesDelta,
    pub pixels_per_point: f32,
}

struct GpuLayer {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

struct GpuSection {
    layers: [Option<GpuLayer>; 3],
}

#[derive(Default, Clone, Copy, Debug)]
pub struct RenderStats {
    pub sections: usize,
    pub visible: usize,
    pub draws: usize,
}

pub struct Renderer {
    /// `None` when drawing off-screen, for scripted runs.
    surface: Option<wgpu::Surface<'static>>,
    offscreen: Option<wgpu::Texture>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    can_copy_surface: bool,
    depth: wgpu::TextureView,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    atlas_layout: wgpu::BindGroupLayout,
    atlas_bg: Option<wgpu::BindGroup>,
    atlas_view: Option<wgpu::TextureView>,
    atlas_tex: Option<wgpu::Texture>,
    filter: (u32, u16),
    terrain: wgpu::RenderPipeline,
    translucent: wgpu::RenderPipeline,
    entity: wgpu::RenderPipeline,
    entity_translucent: wgpu::RenderPipeline,
    glint_pipe: wgpu::RenderPipeline,
    energy_pipe: wgpu::RenderPipeline,
    portal_pipe: wgpu::RenderPipeline,
    portal_bg: Option<wgpu::BindGroup>,
    energy_creeper_bg: Option<wgpu::BindGroup>,
    energy_wither_bg: Option<wgpu::BindGroup>,
    glint_item_bg: Option<wgpu::BindGroup>,
    glint_armor_bg: Option<wgpu::BindGroup>,
    entity_glow: wgpu::RenderPipeline,
    shadow_pipe: wgpu::RenderPipeline,
    through_pipe: wgpu::RenderPipeline,
    celestial_pipe: wgpu::RenderPipeline,
    clouds_pipe: wgpu::RenderPipeline,
    weather_pipe: wgpu::RenderPipeline,
    rain_bg: Option<wgpu::BindGroup>,
    snow_bg: Option<wgpu::BindGroup>,
    sheet_bg: Option<wgpu::BindGroup>,
    sheet_tex: Option<wgpu::Texture>,
    hand_globals_buf: wgpu::Buffer,
    hand_globals_bg: wgpu::BindGroup,
    quad_vb: wgpu::Buffer,
    quad_vb_cap: usize,
    quad_ib: wgpu::Buffer,
    quad_ib_quads: usize,
    sky: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    solid: wgpu::RenderPipeline,
    instance_buf: wgpu::Buffer,
    instance_cap: usize,
    dyn_buf: wgpu::Buffer,
    dyn_cap: usize,
    sections: HashMap<[i32; 3], GpuSection>,
    pub egui: egui_wgpu::Renderer,
    /// (path, world only)
    capture: Option<(PathBuf, bool)>,
    pub stats: RenderStats,
    pub adapter_info: wgpu::AdapterInfo,
}

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn depth_view(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn offscreen_texture(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("off-screen frame"),
        size: wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

#[derive(Debug, thiserror::Error)]
pub enum RenderInitError {
    #[error("couldn't create a drawing surface for the window: {0}")]
    Surface(String),
    #[error("no compatible graphics adapter (Vulkan, Metal or DirectX 12) was found: {0}")]
    Adapter(String),
    #[error("the graphics driver refused to create a device: {0}")]
    Device(String),
}

impl Renderer {
    pub fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        display: Option<Box<dyn wgpu::wgt::WgpuHasDisplayHandle>>,
        width: u32,
        height: u32,
        vsync: bool,
        low_power: bool,
    ) -> Result<Self, RenderInitError> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.display = display;
        let instance = wgpu::Instance::new(desc);
        let surface = instance.create_surface(target).map_err(|e| RenderInitError::Surface(e.to_string()))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: if low_power { wgpu::PowerPreference::LowPower } else { wgpu::PowerPreference::HighPerformance },
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| RenderInitError::Adapter(e.to_string()))?;
        let adapter_info = adapter.get_info();
        tracing::info!("GPU: {} ({:?}, {:?})", adapter_info.name, adapter_info.backend, adapter_info.device_type);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lattice"),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| RenderInitError::Device(e.to_string()))?;

        let caps = surface.get_capabilities(&adapter);
        // egui expects a non-sRGB target; the world shaders encode gamma themselves.
        let preferred = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm];
        let format = preferred
            .into_iter()
            .find(|f| caps.formats.contains(f))
            .or_else(|| caps.formats.iter().copied().find(|f| !f.is_srgb()))
            .unwrap_or(caps.formats[0]);
        tracing::info!("surface format {format:?} (offered: {:?})", caps.formats);
        if format.is_srgb() {
            tracing::warn!("only sRGB surfaces available; colours will look washed out");
        }
        let can_copy_surface = caps.usages.contains(wgpu::TextureUsages::COPY_SRC);
        let present_mode = if vsync {
            wgpu::PresentMode::AutoVsync
        } else if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if can_copy_surface { wgpu::TextureUsages::COPY_SRC } else { wgpu::TextureUsages::empty() },
            format,
            color_space: Default::default(),
            width: width.max(1),
            height: height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        Ok(Self::build(device, queue, config, Some(surface), None, can_copy_surface, adapter_info))
    }

    /// A renderer with no window: frames are drawn to a texture and can only be captured.
    pub fn new_offscreen(width: u32, height: u32) -> Result<Self, RenderInitError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| RenderInitError::Adapter(e.to_string()))?;
        let adapter_info = adapter.get_info();
        tracing::info!("GPU (off-screen): {} ({:?}, {:?})", adapter_info.name, adapter_info.backend, adapter_info.device_type);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lattice"),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| RenderInitError::Device(e.to_string()))?;
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            color_space: Default::default(),
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
        };
        let texture = offscreen_texture(&device, &config);
        Ok(Self::build(device, queue, config, None, Some(texture), true, adapter_info))
    }

    fn build(
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: wgpu::SurfaceConfiguration,
        surface: Option<wgpu::Surface<'static>>,
        offscreen: Option<wgpu::Texture>,
        can_copy_surface: bool,
        adapter_info: wgpu::AdapterInfo,
    ) -> Self {
        let format = config.format;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/world.wgsl").into()),
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() }],
        });
        let hand_globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hand globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hand_globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: hand_globals_buf.as_entire_binding() }],
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let terrain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(&globals_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });
        let globals_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("globals only"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });

        let vertex_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Unorm8x4, 3 => Unorm8x4];
        let instance_attrs = wgpu::vertex_attr_array![4 => Float32x3];
        let terrain_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &vertex_attrs,
            }),
            Some(wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Instance, attributes: &instance_attrs }),
        ];
        let color_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4];
        let color_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ColorVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &color_attrs,
        })];

        let pipeline = |label: &str,
                        layout: &wgpu::PipelineLayout,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        topology: wgpu::PrimitiveTopology,
                        cull: Option<wgpu::Face>,
                        blend: Option<wgpu::BlendState>,
                        depth_write: bool,
                        depth_compare: wgpu::CompareFunction,
                        bias: wgpu::DepthBiasState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
                primitive: wgpu::PrimitiveState {
                    topology,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(depth_compare),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::{CompareFunction as C, PrimitiveTopology as T};
        let nb = wgpu::DepthBiasState::default();
        // LessEqual: models layer coplanar quads (grass block sides draw a tinted overlay
        // exactly on top of the base texture), so equal depth must pass.
        let terrain = pipeline("terrain", &terrain_layout, "vs_terrain", "fs_terrain", &terrain_buffers, T::TriangleList, Some(wgpu::Face::Back), None, true, C::LessEqual, nb);
        let translucent = pipeline(
            "translucent",
            &terrain_layout,
            "vs_terrain",
            "fs_translucent",
            &terrain_buffers,
            T::TriangleList,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
            C::LessEqual,
            nb,
        );
        let entity_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &vertex_attrs,
        })];
        let entity = pipeline("entity", &terrain_layout, "vs_entity", "fs_entity", &entity_buffers, T::TriangleList, None, None, true, C::LessEqual, nb);
        let entity_translucent = pipeline(
            "entity translucent",
            &terrain_layout,
            "vs_entity",
            "fs_entity_translucent",
            &entity_buffers,
            T::TriangleList,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            true,
            C::LessEqual,
            nb,
        );
        let entity_glow = pipeline(
            "entity glow",
            &terrain_layout,
            "vs_entity",
            "fs_entity_glow",
            &entity_buffers,
            T::TriangleList,
            None,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent::OVER,
            }),
            false,
            C::LessEqual,
            nb,
        );
        let glint_pipe = pipeline(
            "glint",
            &terrain_layout,
            "vs_entity",
            "fs_glint",
            &entity_buffers,
            T::TriangleList,
            None,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent::OVER,
            }),
            false,
            C::LessEqual,
            nb,
        );
        let energy_pipe = pipeline(
            "energy",
            &terrain_layout,
            "vs_entity",
            "fs_energy",
            &entity_buffers,
            T::TriangleList,
            None,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent::OVER,
            }),
            false,
            C::LessEqual,
            nb,
        );
        let portal_pipe = pipeline("end portal", &terrain_layout, "vs_portal", "fs_portal", &entity_buffers, T::TriangleList, None, None, true, C::LessEqual, nb);
        let shadow_pipe = pipeline(
            "shadow",
            &terrain_layout,
            "vs_entity",
            "fs_entity_translucent",
            &entity_buffers,
            T::TriangleList,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
            C::LessEqual,
            nb,
        );
        let through_pipe = pipeline(
            "through",
            &terrain_layout,
            "vs_entity",
            "fs_entity_translucent",
            &entity_buffers,
            T::TriangleList,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
            C::Always,
            nb,
        );
        let celestial_pipe = pipeline(
            "celestial",
            &terrain_layout,
            "vs_sky_quad",
            "fs_celestial",
            &entity_buffers,
            T::TriangleList,
            None,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent::OVER,
            }),
            false,
            C::Always,
            nb,
        );
        let clouds_pipe = pipeline("clouds", &terrain_layout, "vs_entity", "fs_clouds", &entity_buffers, T::TriangleList, None, Some(wgpu::BlendState::ALPHA_BLENDING), true, C::LessEqual, nb);
        let weather_pipe = pipeline("weather", &terrain_layout, "vs_entity", "fs_weather", &entity_buffers, T::TriangleList, None, Some(wgpu::BlendState::ALPHA_BLENDING), false, C::LessEqual, nb);
        let sky = pipeline("sky", &globals_only, "vs_sky", "fs_sky", &[], T::TriangleList, None, None, false, C::Always, nb);
        let lines = pipeline(
            "lines",
            &globals_only,
            "vs_color",
            "fs_color",
            &color_buffers,
            T::LineList,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
            C::LessEqual,
            nb,
        );
        let solid = pipeline(
            "solid",
            &globals_only,
            "vs_color",
            "fs_color",
            &color_buffers,
            T::TriangleList,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
            true,
            C::Less,
            nb,
        );

        let instance_cap = 4096;
        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("section offsets"),
            size: (instance_cap * 12) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dyn_cap = 1 << 16;
        let dyn_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("overlay geometry"),
            size: (dyn_cap * std::mem::size_of::<ColorVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let quad_vb_cap = 1 << 16;
        let quad_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame quads"),
            size: (quad_vb_cap * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let quad_ib_quads = 1usize << 14;
        let quad_ib = quad_indices(&device, quad_ib_quads);
        let egui = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions { depth_stencil_format: Some(DEPTH), ..Default::default() });
        let depth = depth_view(&device, config.width, config.height);
        Renderer {
            surface,
            offscreen,
            device,
            queue,
            config,
            can_copy_surface,
            depth,
            globals_buf,
            globals_bg,
            atlas_layout,
            atlas_bg: None,
            atlas_view: None,
            atlas_tex: None,
            filter: (MIP_LEVELS - 1, 1),
            terrain,
            translucent,
            entity,
            entity_translucent,
            glint_pipe,
            energy_pipe,
            portal_pipe,
            portal_bg: None,
            energy_creeper_bg: None,
            energy_wither_bg: None,
            glint_item_bg: None,
            glint_armor_bg: None,
            entity_glow,
            shadow_pipe,
            through_pipe,
            celestial_pipe,
            clouds_pipe,
            weather_pipe,
            rain_bg: None,
            snow_bg: None,
            sheet_bg: None,
            sheet_tex: None,
            hand_globals_buf,
            hand_globals_bg,
            quad_vb,
            quad_vb_cap,
            quad_ib,
            quad_ib_quads,
            sky,
            lines,
            solid,
            instance_buf,
            instance_cap,
            dyn_buf,
            dyn_cap,
            sections: HashMap::new(),
            egui,
            capture: None,
            stats: RenderStats::default(),
            adapter_info,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 || (w, h) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
        if self.offscreen.is_some() {
            self.offscreen = Some(offscreen_texture(&self.device, &self.config));
        }
        self.depth = depth_view(&self.device, w, h);
    }

    pub fn set_vsync(&mut self, vsync: bool) {
        self.config.present_mode = if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
    }

    pub fn upload_atlas(&mut self, atlas: &Atlas) {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("block atlas"),
            size: wgpu::Extent3d { width: atlas.size, height: atlas.size, depth_or_array_layers: 1 },
            mip_level_count: MIP_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, img) in atlas.mips.iter().enumerate() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                img.as_raw(),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(img.width() * 4), rows_per_image: Some(img.height()) },
                wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 },
            );
        }
        self.atlas_view = Some(tex.create_view(&Default::default()));
        self.atlas_tex = Some(tex);
        self.rebuild_atlas_binding();
    }

    /// Replaces a sprite's picture in the block atlas, one image per mip level.
    pub fn write_atlas(&mut self, x: u32, y: u32, levels: &[image::RgbaImage]) {
        let Some(tex) = &self.atlas_tex else { return };
        for (level, img) in levels.iter().enumerate() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: tex, mip_level: level as u32, origin: wgpu::Origin3d { x: x >> level, y: y >> level, z: 0 }, aspect: wgpu::TextureAspect::All },
                img.as_raw(),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(img.width() * 4), rows_per_image: Some(img.height()) },
                wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 },
            );
        }
    }

    /// The sheet of mob and block entity textures. Never filtered: these are drawn big and blocky.
    pub fn upload_sheet(&mut self, sheet: &Sheet, image: &image::RgbaImage) {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("entity sheet"),
            size: wgpu::Extent3d { width: sheet.width, height: sheet.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            image.as_raw(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(sheet.width * 4), rows_per_image: Some(sheet.height) },
            wgpu::Extent3d { width: sheet.width, height: sheet.height, depth_or_array_layers: 1 },
        );
        let view = tex.create_view(&Default::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("entity sheet"), mag_filter: wgpu::FilterMode::Nearest, min_filter: wgpu::FilterMode::Nearest, ..Default::default() });
        self.sheet_bg = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("entity sheet"),
            layout: &self.atlas_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        }));
        self.sheet_tex = Some(tex);
    }

    /// A picture that repeats, so it can't share a sheet.
    fn repeating(&self, img: &image::RgbaImage, label: &str) -> wgpu::BindGroup {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            img.as_raw(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(img.width() * 4), rows_per_image: None },
            wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 },
        );
        let view = tex.create_view(&Default::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(label),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.atlas_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        })
    }

    /// The pictures of enchantment shine, which repeat too.
    pub fn upload_glint(&mut self, item: &image::RgbaImage, armor: &image::RgbaImage) {
        self.glint_item_bg = Some(self.repeating(item, "glint"));
        self.glint_armor_bg = Some(self.repeating(armor, "glint"));
    }

    /// The End portal's stars, and the energy swirling over charged creepers and withers.
    pub fn upload_effects(&mut self, portal: &image::RgbaImage, creeper: &image::RgbaImage, wither: &image::RgbaImage) {
        self.portal_bg = Some(self.repeating(portal, "end portal"));
        self.energy_creeper_bg = Some(self.repeating(creeper, "energy"));
        self.energy_wither_bg = Some(self.repeating(wither, "energy"));
    }

    /// Rain and snow pictures, which repeat, so they can't share a sheet.
    pub fn upload_weather(&mut self, rain: &image::RgbaImage, snow: &image::RgbaImage) {
        let make = |img: &image::RgbaImage| {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("weather"),
                size: wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                img.as_raw(),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(img.width() * 4), rows_per_image: Some(img.height()) },
                wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 },
            );
            let view = tex.create_view(&Default::default());
            let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("weather"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            });
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("weather"),
                layout: &self.atlas_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
            })
        };
        self.rain_bg = Some(make(rain));
        self.snow_bg = Some(make(snow));
    }

    /// Replaces part of the entity sheet, for a skin that just arrived.
    pub fn write_sheet(&mut self, x: u32, y: u32, img: &image::RgbaImage) {
        let Some(tex) = &self.sheet_tex else { return };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            img.as_raw(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(img.width() * 4), rows_per_image: Some(img.height()) },
            wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 },
        );
    }

    /// Mipmap levels used, and anisotropic filtering (1 is off).
    pub fn set_filtering(&mut self, mip_levels: u32, anisotropy: u16) {
        if self.filter != (mip_levels, anisotropy) {
            self.filter = (mip_levels, anisotropy);
            self.rebuild_atlas_binding();
        }
    }

    fn rebuild_atlas_binding(&mut self) {
        let Some(view) = &self.atlas_view else { return };
        let (mips, aniso) = self.filter;
        // Anisotropic filtering needs linear filters everywhere; without it blocks keep their pixels.
        let linear = aniso > 1;
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            mag_filter: if linear { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest },
            min_filter: if linear { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest },
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            lod_max_clamp: mips as f32,
            anisotropy_clamp: aniso.max(1),
            ..Default::default()
        });
        self.atlas_bg = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas"),
            layout: &self.atlas_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        }));
    }

    pub fn upload_section(&mut self, mesh: SectionMesh) {
        if mesh.is_empty() {
            self.sections.remove(&mesh.origin);
            return;
        }
        let layers = mesh.layers.map(|l| {
            if l.indices.is_empty() {
                return None;
            }
            Some(GpuLayer {
                vertices: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&l.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                indices: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&l.indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                count: l.indices.len() as u32,
            })
        });
        self.sections.insert(mesh.origin, GpuSection { layers });
    }

    pub fn remove_column(&mut self, x: i32, z: i32) {
        self.sections.retain(|k, _| !(k[0] == x && k[2] == z));
    }

    pub fn clear_world(&mut self) {
        self.sections.clear();
    }

    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Waits for the graphics card to finish everything it's been given.
    pub fn wait_idle(&self) {
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(5)) });
    }

    /// Saves the next presented frame as a PNG.
    pub fn capture_next_frame(&mut self, path: PathBuf) {
        self.capture = Some((path, false));
    }

    /// Saves the next frame's 3D view without any interface on top.
    pub fn capture_next_world(&mut self, path: PathBuf) {
        self.capture = Some((path, true));
    }

    fn begin_pass(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, depth: &wgpu::TextureView, clear: Option<[f64; 3]>) -> wgpu::RenderPass<'static> {
        encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: match clear {
                            Some(c) => wgpu::LoadOp::Clear(wgpu::Color { r: c[0], g: c[1], b: c[2], a: 1.0 }),
                            None => wgpu::LoadOp::Load,
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime()
    }

    fn encode_readback(&self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) -> (wgpu::Buffer, u32, u32, u32) {
        let (w, h) = (self.config.width, self.config.height);
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        (buf, row, w, h)
    }

    /// The combined matrix, and one that turns a screen position (x, y, 1, 0) into a view direction
    /// for the sky. Worked out directly, as inverting the whole projection loses too much precision.
    fn projection(&self, cam: &Camera, far: f32) -> (Mat4, Mat4) {
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let turn = Mat4::from_rotation_z(cam.roll.to_radians()) * Mat4::from_rotation_x(cam.nod.to_radians()) * cam.turn();
        let view = Mat4::from_rotation_z(cam.roll.to_radians()) * Mat4::from_rotation_x(cam.nod.to_radians()) * Mat4::from_translation(cam.shift) * cam.turn();
        let mut proj = Mat4::perspective_rh(cam.fov_y_deg.to_radians(), aspect, 0.05, far);
        if let Some((spin, squeeze)) = cam.warp {
            let axis = glam::Vec3::new(0.0, std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
            let turn = spin.to_radians();
            proj = proj * Mat4::from_axis_angle(axis, turn) * Mat4::from_scale(glam::Vec3::new(1.0 / squeeze.max(0.01), 1.0, 1.0)) * Mat4::from_axis_angle(axis, -turn);
        }
        let tan_y = (cam.fov_y_deg.to_radians() * 0.5).tan();
        let back = turn.transpose();
        let (right, up, ahead) = (back.x_axis.truncate(), back.y_axis.truncate(), -back.z_axis.truncate());
        let rays = Mat4::from_cols((right * tan_y * aspect).extend(0.0), (up * tan_y).extend(0.0), ahead.extend(0.0), Vec4::ZERO);
        (proj * view, rays)
    }

    /// Draws one frame. `world` is `None` on menu screens (only egui is drawn).
    pub fn render(&mut self, world: Option<&FrameWorld>, ui: EguiFrame, clear: [f64; 3]) {
        // Texture changes apply even if this frame can't be presented.
        for (id, deltas) in &ui.textures.set {
            for delta in deltas {
                self.egui.update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        for id in &ui.textures.free {
            self.egui.free_texture(id);
        }
        ui.textures.clear();
        let frame = match &self.surface {
            Some(surface) => match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => Some(f),
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    surface.configure(&self.device, &self.config);
                    return;
                }
                other => {
                    tracing::debug!("skipping frame: {other:?}");
                    return;
                }
            },
            None => None,
        };
        let texture = match (&frame, &self.offscreen) {
            (Some(f), _) => f.texture.clone(),
            (None, Some(t)) => t.clone(),
            (None, None) => return,
        };
        let view = texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());

        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [self.config.width, self.config.height], pixels_per_point: ui.pixels_per_point };
        let egui_cmds = self.egui.update_buffers(&self.device, &self.queue, &mut encoder, ui.primitives, &screen);

        // ---- world
        let mut draw_list: Vec<([i32; 3], f32)> = Vec::new();
        let mut quad_runs = [(0i32, 0u32); 21];
        if let Some(w) = world {
            let (vp, inv) = self.projection(w.camera, (w.fog_end * 1.5).max(1000.0));
            let globals = Globals {
                view_proj: vp.to_cols_array_2d(),
                inv_view_proj: inv.to_cols_array_2d(),
                sky_zenith: [w.sky_zenith[0], w.sky_zenith[1], w.sky_zenith[2], 1.0],
                sky_horizon: [w.sky_horizon[0], w.sky_horizon[1], w.sky_horizon[2], 1.0],
                sun: [w.sun_dir.x, w.sun_dir.y, w.sun_dir.z, w.daylight],
                fog: [w.fog_start, w.fog_end, w.min_brightness, w.gamma],
            };
            self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

            let planes = frustum_planes(vp);
            let cam = w.camera.pos;
            let max_d2 = ((w.fog_end + 24.0) as f64).powi(2);
            for key in self.sections.keys() {
                let min = DVec3::new(key[0] as f64 * 16.0, key[1] as f64 * 16.0, key[2] as f64 * 16.0) - cam;
                let centre = min + DVec3::splat(8.0);
                if centre.x * centre.x + centre.z * centre.z > max_d2 {
                    continue;
                }
                let (mn, mx) = (min.as_vec3(), (min + DVec3::splat(16.0)).as_vec3());
                if aabb_in_frustum(&planes, mn, mx) {
                    draw_list.push((*key, centre.length_squared() as f32));
                }
            }
            draw_list.sort_by(|a, b| a.1.total_cmp(&b.1));
            if draw_list.len() > self.instance_cap {
                self.instance_cap = draw_list.len().next_power_of_two();
                self.instance_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("section offsets"),
                    size: (self.instance_cap * 12) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            let offsets: Vec<[f32; 3]> = draw_list
                .iter()
                .map(|(k, _)| {
                    let o = DVec3::new(k[0] as f64 * 16.0, k[1] as f64 * 16.0, k[2] as f64 * 16.0) - cam;
                    [o.x as f32, o.y as f32, o.z as f32]
                })
                .collect();
            self.queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(&offsets));

            let n = w.lines.len() + w.triangles.len();
            if n > self.dyn_cap {
                self.dyn_cap = n.next_power_of_two();
                self.dyn_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("overlay geometry"),
                    size: (self.dyn_cap * std::mem::size_of::<ColorVertex>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            self.queue.write_buffer(&self.dyn_buf, 0, bytemuck::cast_slice(w.triangles));
            self.queue.write_buffer(&self.dyn_buf, (w.triangles.len() * std::mem::size_of::<ColorVertex>()) as u64, bytemuck::cast_slice(w.lines));

            // Moving geometry: one vertex buffer, with a run for each list.
            let b = w.batches;
            let lists = [&b.entity, &b.blocks, &b.entity_translucent, &b.blocks_translucent, &b.entity_glow, &b.hand_entity, &b.hand_blocks, &b.shadow, &b.through, &b.celestial, &b.clouds, &b.rain, &b.snow, &b.overlay, &b.overlay_blocks, &b.glint_item, &b.glint_armor, &b.hand_glint, &b.end_portal, &b.energy_creeper, &b.energy_wither];
            let total: usize = lists.iter().map(|l| l.len()).sum();
            if total > self.quad_vb_cap {
                self.quad_vb_cap = total.next_power_of_two();
                self.quad_vb = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("frame quads"),
                    size: (self.quad_vb_cap * std::mem::size_of::<Vertex>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            let most = lists.iter().map(|l| l.len() / 4).max().unwrap_or(0);
            if most > self.quad_ib_quads {
                self.quad_ib_quads = most.next_power_of_two();
                self.quad_ib = quad_indices(&self.device, self.quad_ib_quads);
            }
            let mut at = 0usize;
            for (i, l) in lists.iter().enumerate() {
                quad_runs[i] = (at as i32, (l.len() / 4 * 6) as u32);
                if !l.is_empty() {
                    self.queue.write_buffer(&self.quad_vb, (at * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(l));
                }
                at += l.len();
            }

            // The held item has its own field of view and doesn't turn with the camera.
            let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
            let cam = w.camera;
            let view = Mat4::from_rotation_z(cam.roll.to_radians()) * Mat4::from_rotation_x(cam.nod.to_radians()) * Mat4::from_translation(cam.shift);
            let hand_vp = Mat4::perspective_rh(w.hand_fov.to_radians(), aspect, 0.05, 16.0) * view;
            let hand = Globals {
                view_proj: hand_vp.to_cols_array_2d(),
                inv_view_proj: hand_vp.inverse().to_cols_array_2d(),
                sky_zenith: globals.sky_zenith,
                sky_horizon: globals.sky_horizon,
                sun: globals.sun,
                // No fog this close to the eye.
                fog: [1000.0, 2000.0, globals.fog[2], globals.fog[3]],
            };
            self.queue.write_buffer(&self.hand_globals_buf, 0, bytemuck::bytes_of(&hand));
        }

        let eight_bit = matches!(
            self.config.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
        );
        let capture = self.capture.take().filter(|_| self.can_copy_surface && eight_bit);
        let world_only = capture.as_ref().is_some_and(|(_, w)| *w);
        let mut readback = None;
        {
            let mut pass = Self::begin_pass(&mut encoder, &view, &self.depth, Some(clear));
            let mut draws = 0;
            if let (Some(w), Some(atlas)) = (world, &self.atlas_bg) {
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_pipeline(&self.sky);
                pass.draw(0..3, 0..1);
                if quad_runs[9].1 > 0 {
                    if let Some(sheet) = &self.sheet_bg {
                        pass.set_pipeline(&self.celestial_pipe);
                        pass.set_bind_group(1, sheet, &[]);
                        pass.set_index_buffer(self.quad_ib.slice(..), wgpu::IndexFormat::Uint32);
                        pass.set_vertex_buffer(0, self.quad_vb.slice(..));
                        pass.draw_indexed(0..quad_runs[9].1, quad_runs[9].0, 0..1);
                    }
                }

                pass.set_pipeline(&self.terrain);
                pass.set_bind_group(1, atlas, &[]);
                pass.set_vertex_buffer(1, self.instance_buf.slice(..));
                for layer in [0usize, 1] {
                    for (i, (k, _)) in draw_list.iter().enumerate() {
                        if let Some(l) = self.sections.get(k).and_then(|s| s.layers[layer].as_ref()) {
                            pass.set_vertex_buffer(0, l.vertices.slice(..));
                            pass.set_index_buffer(l.indices.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..l.count, 0, i as u32..i as u32 + 1);
                            draws += 1;
                        }
                    }
                }
                if !w.triangles.is_empty() {
                    pass.set_pipeline(&self.solid);
                    pass.set_vertex_buffer(0, self.dyn_buf.slice(..));
                    pass.draw(0..w.triangles.len() as u32, 0..1);
                }
                let sheet = self.sheet_bg.as_ref().unwrap_or(atlas);
                pass.set_index_buffer(self.quad_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.quad_vb.slice(..));
                let run = |pass: &mut wgpu::RenderPass<'_>, r: (i32, u32)| {
                    if r.1 > 0 {
                        pass.draw_indexed(0..r.1, r.0, 0..1);
                    }
                };
                pass.set_pipeline(&self.entity);
                pass.set_bind_group(1, sheet, &[]);
                run(&mut pass, quad_runs[0]);
                if let (true, Some(portal)) = (quad_runs[18].1 > 0, &self.portal_bg) {
                    pass.set_pipeline(&self.portal_pipe);
                    pass.set_bind_group(1, portal, &[]);
                    run(&mut pass, quad_runs[18]);
                    pass.set_pipeline(&self.entity);
                    pass.set_bind_group(1, sheet, &[]);
                }
                pass.set_bind_group(1, atlas, &[]);
                run(&mut pass, quad_runs[1]);
                pass.set_pipeline(&self.shadow_pipe);
                pass.set_bind_group(1, sheet, &[]);
                run(&mut pass, quad_runs[7]);

                pass.set_pipeline(&self.translucent);
                pass.set_bind_group(1, atlas, &[]);
                pass.set_vertex_buffer(1, self.instance_buf.slice(..));
                for (i, (k, _)) in draw_list.iter().enumerate().rev() {
                    if let Some(l) = self.sections.get(k).and_then(|s| s.layers[2].as_ref()) {
                        pass.set_vertex_buffer(0, l.vertices.slice(..));
                        pass.set_index_buffer(l.indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..l.count, 0, i as u32..i as u32 + 1);
                        draws += 1;
                    }
                }
                pass.set_index_buffer(self.quad_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.quad_vb.slice(..));
                pass.set_pipeline(&self.entity_translucent);
                pass.set_bind_group(1, sheet, &[]);
                run(&mut pass, quad_runs[2]);
                pass.set_bind_group(1, atlas, &[]);
                run(&mut pass, quad_runs[3]);
                pass.set_pipeline(&self.entity_glow);
                pass.set_bind_group(1, sheet, &[]);
                run(&mut pass, quad_runs[4]);
                pass.set_pipeline(&self.clouds_pipe);
                run(&mut pass, quad_runs[10]);
                pass.set_pipeline(&self.weather_pipe);
                if let Some(rain) = &self.rain_bg {
                    pass.set_bind_group(1, rain, &[]);
                    run(&mut pass, quad_runs[11]);
                }
                if let Some(snow) = &self.snow_bg {
                    pass.set_bind_group(1, snow, &[]);
                    run(&mut pass, quad_runs[12]);
                }
                pass.set_pipeline(&self.energy_pipe);
                if let Some(g) = &self.energy_creeper_bg {
                    pass.set_bind_group(1, g, &[]);
                    run(&mut pass, quad_runs[19]);
                }
                if let Some(g) = &self.energy_wither_bg {
                    pass.set_bind_group(1, g, &[]);
                    run(&mut pass, quad_runs[20]);
                }
                pass.set_pipeline(&self.glint_pipe);
                if let Some(g) = &self.glint_item_bg {
                    pass.set_bind_group(1, g, &[]);
                    run(&mut pass, quad_runs[15]);
                }
                if let Some(g) = &self.glint_armor_bg {
                    pass.set_bind_group(1, g, &[]);
                    run(&mut pass, quad_runs[16]);
                }
                pass.set_bind_group(1, sheet, &[]);
                pass.set_pipeline(&self.through_pipe);
                run(&mut pass, quad_runs[8]);
                if !w.lines.is_empty() && !world_only {
                    pass.set_pipeline(&self.lines);
                    pass.set_vertex_buffer(0, self.dyn_buf.slice(..));
                    let start = w.triangles.len() as u32;
                    pass.draw(start..start + w.lines.len() as u32, 0..1);
                }
            }
            self.stats = RenderStats { sections: self.sections.len(), visible: draw_list.len(), draws };
            // The held item on top of everything, with depth starting over.
            if (quad_runs[5].1 > 0 || quad_runs[6].1 > 0) && self.atlas_bg.is_some() {
                drop(pass);
                pass = Self::begin_pass(&mut encoder, &view, &self.depth, None);
                pass.set_bind_group(0, &self.hand_globals_bg, &[]);
                pass.set_index_buffer(self.quad_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.quad_vb.slice(..));
                pass.set_pipeline(&self.entity);
                if quad_runs[5].1 > 0 {
                    pass.set_bind_group(1, self.sheet_bg.as_ref().or(self.atlas_bg.as_ref()).unwrap(), &[]);
                    pass.draw_indexed(0..quad_runs[5].1, quad_runs[5].0, 0..1);
                }
                if quad_runs[6].1 > 0 {
                    pass.set_bind_group(1, self.atlas_bg.as_ref().unwrap(), &[]);
                    pass.draw_indexed(0..quad_runs[6].1, quad_runs[6].0, 0..1);
                }
                if let (true, Some(g)) = (quad_runs[17].1 > 0, &self.glint_item_bg) {
                    pass.set_pipeline(&self.glint_pipe);
                    pass.set_bind_group(1, g, &[]);
                    pass.draw_indexed(0..quad_runs[17].1, quad_runs[17].0, 0..1);
                }
            }
            // Then whatever's over the whole view.
            if (quad_runs[13].1 > 0 || quad_runs[14].1 > 0) && self.atlas_bg.is_some() {
                drop(pass);
                pass = Self::begin_pass(&mut encoder, &view, &self.depth, None);
                pass.set_bind_group(0, &self.hand_globals_bg, &[]);
                pass.set_index_buffer(self.quad_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.quad_vb.slice(..));
                pass.set_pipeline(&self.entity_translucent);
                if quad_runs[14].1 > 0 {
                    pass.set_bind_group(1, self.atlas_bg.as_ref().unwrap(), &[]);
                    pass.draw_indexed(0..quad_runs[14].1, quad_runs[14].0, 0..1);
                }
                if quad_runs[13].1 > 0 {
                    pass.set_bind_group(1, self.sheet_bg.as_ref().or(self.atlas_bg.as_ref()).unwrap(), &[]);
                    pass.draw_indexed(0..quad_runs[13].1, quad_runs[13].0, 0..1);
                }
            }
            if world_only {
                // Copy the 3D view, then continue with the interface in a second pass.
                drop(pass);
                readback = Some(self.encode_readback(&mut encoder, &texture));
                pass = Self::begin_pass(&mut encoder, &view, &self.depth, None);
            }
            self.egui.render(&mut pass, ui.primitives, &screen);
        }
        if capture.is_some() && !world_only {
            readback = Some(self.encode_readback(&mut encoder, &texture));
        }

        self.queue.submit(egui_cmds.into_iter().chain(std::iter::once(encoder.finish())));
        if let (Some((path, _)), Some((buf, row, w, h))) = (capture, readback) {
            self.save_capture(&buf, row, w, h, &path);
        }
        if let Some(frame) = frame {
            self.queue.present(frame);
        }
    }

    fn save_capture(&self, buf: &wgpu::Buffer, row: u32, w: u32, h: u32, path: &std::path::Path) {
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(5)) });
        let Ok(data) = slice.get_mapped_range() else {
            tracing::warn!("couldn't read back the frame");
            return;
        };
        let bgra = matches!(self.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut img = image::RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = (y * row + x * 4) as usize;
                let p = &data[i..i + 4];
                let px = if bgra { [p[2], p[1], p[0], 255] } else { [p[0], p[1], p[2], 255] };
                img.put_pixel(x, y, image::Rgba(px));
            }
        }
        drop(data);
        buf.unmap();
        match img.save(path) {
            Ok(()) => tracing::info!("saved frame to {}", path.display()),
            Err(e) => tracing::warn!("couldn't save frame: {e}"),
        }
    }
}

/// Index list for a run of quads, each as two triangles.
fn quad_indices(device: &wgpu::Device, quads: usize) -> wgpu::Buffer {
    let mut list = Vec::with_capacity(quads * 6);
    for q in 0..quads as u32 {
        let b = q * 4;
        list.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 3, b]);
    }
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("quad indices"), contents: bytemuck::cast_slice(&list), usage: wgpu::BufferUsages::INDEX })
}

fn frustum_planes(m: Mat4) -> [Vec4; 6] {
    let r = |i: usize| m.row(i);
    let planes = [r(3) + r(0), r(3) - r(0), r(3) + r(1), r(3) - r(1), r(2), r(3) - r(2)];
    planes.map(|p| p / p.truncate().length())
}

fn aabb_in_frustum(planes: &[Vec4; 6], min: Vec3, max: Vec3) -> bool {
    planes.iter().all(|p| {
        let v = Vec3::new(if p.x >= 0.0 { max.x } else { min.x }, if p.y >= 0.0 { max.y } else { min.y }, if p.z >= 0.0 { max.z } else { min.z });
        p.truncate().dot(v) + p.w >= 0.0
    })
}

#[cfg(test)]
mod camera_tests {
    use super::*;

    fn camera(yaw: f32, pitch: f32) -> Camera {
        Camera { pos: Default::default(), yaw, pitch, fov_y_deg: 70.0, roll: 0.0, nod: 0.0, shift: Vec3::ZERO, warp: None }
    }

    #[test]
    fn the_view_turns_the_way_the_camera_looks() {
        for yaw in (-360..=360).step_by(45) {
            for pitch in [-89.0f32, -45.0, -10.0, 0.0, 10.0, 45.0, 89.0] {
                let c = camera(yaw as f32, pitch);
                let expected = Mat4::look_to_rh(Vec3::ZERO, c.forward(), Vec3::Y);
                assert!(c.turn().abs_diff_eq(expected, 1e-4), "yaw {yaw} pitch {pitch}");
            }
        }
    }

    #[test]
    fn looking_straight_up_or_down_doesnt_flip_the_view() {
        for yaw in (-720..=720).step_by(7) {
            for (at, near) in [(90.0f32, 89.9f32), (-90.0, -89.9)] {
                let (a, b) = (camera(yaw as f32, at).turn(), camera(yaw as f32, near).turn());
                assert!(a.abs_diff_eq(b, 0.01), "yaw {yaw}: the view jumps between pitch {near} and {at}");
                assert!(a.is_finite());
            }
        }
    }
}
