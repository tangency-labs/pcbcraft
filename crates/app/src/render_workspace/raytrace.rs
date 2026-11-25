//! GPU path tracing for the live board camera.

use super::{
    native::{Progress, Request},
    scene::{self, Material},
};
use crate::editor_layout::HostId;
use eframe::egui_wgpu::{RenderState, wgpu};
use egui::{ColorImage, Context};
use pcbcraft_engine::Project;
use solidcore_record::Vec3;
use solidcraft_geom::{Aabb3, Mesh};
use solidcraft_render::{
    ProductMesh, ProductScene,
    job::{CancelToken, DirectionalLightOptions, DirectionalLights, ImageHost, ImageRequest, JobId, JobOptions, Phase, StudioLighting},
    lighting::environment::EnvironmentSource,
};
use solidcraft_viewport::{
    Camera,
    surface::{OpaqueSurface, srgb8_to_linear},
};
use std::{
    f64::consts::{FRAC_PI_2, PI, TAU},
    sync::{Arc, Mutex, OnceLock, Weak, mpsc},
    time::Duration,
};

pub(super) type Outcome = Result<ColorImage, String>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pose {
    pub camera: Camera,
    pub aspect: f32,
}

struct Pending {
    request: Request,
    cancel: CancelToken,
    receiver: mpsc::Receiver<Outcome>,
    progress: Progress,
}

/// One GPU frame at a time for the entire editor. The slot is held until the worker exits,
/// including after cancellation, so repeated clicks or closing panes cannot accumulate threads.
#[derive(Default)]
pub(super) struct Worker {
    backend: Option<Arc<Backend>>,
    pending: Option<Pending>,
    serial: u64,
}

/// The path tracer owns an independent device on the application's hardware adapter. Its
/// pipeline is initialized by the render worker, and reused until the editor closes.
struct Backend {
    adapter: wgpu::Adapter,
    host: OnceLock<Result<ImageHost, String>>,
    scene: Mutex<Option<CachedScene>>,
    studio: OnceLock<Result<Arc<EnvironmentSource>, String>>,
}

struct CachedScene {
    request: Request,
    // Preserve allocation identity without keeping the board snapshot alive.
    project: Weak<Project>,
    scene: ProductScene,
}

impl CachedScene {
    fn matches(&self, request: Request, project: &Arc<Project>) -> bool {
        self.request == request && std::ptr::eq(self.project.as_ptr(), Arc::as_ptr(project))
    }
}

impl Worker {
    /// Discover a compatible hardware adapter from the running viewport without creating a
    /// ray-tracing device. Windows retains the viewport's Vulkan-only safety boundary.
    pub(super) fn new(render: Option<&RenderState>) -> Self {
        let backend = render
            .and_then(|render| {
                if compatible(&render.adapter) {
                    return Some(&render.adapter);
                }
                if cfg!(target_os = "windows") {
                    return None;
                }
                render
                    .available_adapters
                    .iter()
                    .filter(|adapter| compatible(adapter))
                    .max_by_key(|adapter| u8::from(adapter.get_info().device_type == wgpu::DeviceType::DiscreteGpu))
            })
            .map(|adapter| Arc::new(Backend { adapter: adapter.clone(), host: OnceLock::new(), scene: Mutex::new(None), studio: OnceLock::new() }));
        Self { backend, pending: None, serial: 0 }
    }

    /// Render is available only for hardware that can open a Vulkan or Metal path device.
    pub(super) fn available(&self) -> bool {
        self.backend.as_ref().is_some_and(|backend| !matches!(backend.host.get(), Some(Err(_))))
    }

    pub(super) fn progress(&self, request: Request) -> Option<&'static str> {
        let pending = self.pending.as_ref()?;
        (pending.request == request && !pending.cancel.is_cancelled()).then(|| pending.progress.message()).flatten()
    }

    /// Return the current owner and captured board revision, while the worker holds its slot.
    pub(super) fn request(&self) -> Option<Request> {
        self.pending.as_ref().map(|pending| pending.request)
    }

    /// Cancel the current job only when the caller owns its captured request.
    pub(super) fn cancel(&self, request: Request) {
        if let Some(pending) = &self.pending
            && pending.request == request
        {
            pending.cancel.cancel();
        }
    }

    /// Start a frame from an immutable board and a copied camera. Geometry and image encoding
    /// both run off the editor thread. Cancelled results stay local to their original pane.
    pub(super) fn start(&mut self, ctx: &Context, host: HostId, request: Request, project: Arc<Project>, pose: Pose) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("Another ray-traced frame is rendering.".into());
        }
        if !self.available() {
            return Err("GPU ray tracing requires a compatible hardware Vulkan or Metal adapter.".into());
        }
        let backend = self.backend.clone().ok_or("GPU ray tracing is unavailable.")?;
        self.serial = self.serial.checked_add(1).ok_or("Too many board images rendered in this session.")?;
        let serial = self.serial;
        let (sender, receiver) = mpsc::channel();
        let cancel = CancelToken::default();
        let background_cancel = cancel.clone();
        let message = if backend.host.get().is_some() { "Preparing ray-traced frame…" } else { "Preparing ray-tracing shaders…" };
        let progress = Progress::new(ctx, host, message);
        let background = progress.clone();
        std::thread::Builder::new()
            .name("pcbcraft-ray-trace".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let host = backend.host.get_or_init(|| solidcraft_gpu::path::host_on_adapter(&backend.adapter).map_err(|e| e.to_string()));
                    let host = host.as_ref().map_err(Clone::clone)?;
                    let scene = {
                        let mut cached = backend.scene.lock().map_err(|_| "The ray-traced scene cache is unavailable.")?;
                        if cached.as_ref().is_none_or(|entry| !entry.matches(request, &project)) {
                            background.report("Building ray-tracing geometry…");
                            let scene = build_scene(&project, request, &background_cancel)?;
                            *cached = Some(CachedScene { request, project: Arc::downgrade(&project), scene });
                        }
                        cached.as_ref().ok_or("The ray-traced scene is unavailable.")?.scene.clone()
                    };
                    let environment = backend.studio.get_or_init(studio_environment).as_ref().map_err(Clone::clone)?;
                    let frame = Frame { scene, pose, environment: Some(Arc::clone(environment)) };
                    render(host, serial, request.revision, frame, background_cancel, |message| background.report(message))
                }))
                .unwrap_or_else(|_| Err("The ray-traced render stopped unexpectedly.".into()));
                let _ = sender.send(result);
                background.repaint();
            })
            .map_err(|error| format!("Could not start ray tracing. {error}"))?;
        self.pending = Some(Pending { request, cancel, receiver, progress });
        Ok(())
    }

    /// Return completed work and release the slot. Explicitly cancelled images are discarded
    /// even when cancellation races with the worker's final publication.
    pub(super) fn poll(&mut self) -> Option<(Request, Option<Outcome>)> {
        let pending = self.pending.as_ref()?;
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err("The ray-traced render stopped unexpectedly.".into()),
        };
        let result = (pending.request, (!pending.cancel.is_cancelled()).then_some(result));
        self.pending = None;
        Some(result)
    }
}

impl Drop for Worker {
    /// Dropping the editor cancels work without joining a busy render thread.
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancel.cancel();
        }
    }
}

/// Software Vulkan devices must never enable the Render action.
fn hardware(device_type: wgpu::DeviceType) -> bool {
    matches!(device_type, wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::VirtualGpu)
}

/// Limit ray tracing to hardware adapters on a supported compute backend.
fn supports(info: &wgpu::AdapterInfo) -> bool {
    hardware(info.device_type) && matches!(info.backend, wgpu::Backend::Vulkan | wgpu::Backend::Metal)
}

/// Match the path device's storage-buffer requirement before showing Render as enabled.
fn compatible(adapter: &wgpu::Adapter) -> bool {
    supports(&adapter.get_info()) && adapter.limits().max_storage_buffers_per_shader_stage >= 6
}

/// Keep the live camera aspect at a bounded rendering resolution.
fn image_size(aspect: f32, edge: f32) -> [u32; 2] {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 4.0 / 3.0 };
    if aspect >= 1.0 { [edge as u32, (edge / aspect).round().max(1.0) as u32] } else { [(edge * aspect).round().max(1.0) as u32, edge as u32] }
}

/// Convert the board once per snapshot. Camera-only changes can share its immutable meshes.
fn build_scene(project: &Project, request: Request, cancel: &CancelToken) -> Result<ProductScene, String> {
    cancel.check().map_err(|err| err.to_string())?;
    let board = project.board_view(request.target).ok_or("This board is no longer available.")?;
    let (mesh, thickness) = pcbcraft_board_solid::display_board(board, 10_000).map_err(|err| err.to_string())?;
    let mut substrate = Mesh::default();
    let mut copper = Mesh::default();
    let mut bounds = Aabb3::EMPTY;
    scene::visit(mesh, board, thickness.nm(), super::copper::RAYTRACE_OFFSET_MM, |material, points, normal| {
        let output = match material {
            Material::Substrate => &mut substrate,
            Material::Copper => &mut copper,
        };
        let vertex = output.positions.len() as u32;
        output.positions.extend(points);
        output.normals.extend([normal; 3]);
        output.triangles.push([vertex, vertex + 1, vertex + 2]);
        output.tri_face.push(0);
        for point in points {
            bounds.add(point);
        }
    })?;
    cancel.check().map_err(|err| err.to_string())?;
    let mut meshes = vec![material_mesh(Material::Substrate, substrate)?];
    if !copper.triangles.is_empty() {
        meshes.push(material_mesh(Material::Copper, copper)?);
    }
    Ok(ProductScene { bounds, meshes })
}

/// Bake a small, world-fixed studio with two upper softboxes and weaker lower fill.
fn studio_environment() -> Result<Arc<EnvironmentSource>, String> {
    const WIDTH: usize = 192;
    const HEIGHT: usize = 96;
    let mut pixels = Vec::with_capacity(WIDTH * HEIGHT);
    for row in 0..HEIGHT {
        let elevation = FRAC_PI_2 - PI * (row as f64 + 0.5) / HEIGHT as f64;
        for column in 0..WIDTH {
            let azimuth = TAU * (column as f64 + 0.5) / WIDTH as f64;
            let radiance = 0.14
                + 0.18 * elevation.sin().max(0.0)
                + 1.8 * softbox(azimuth, elevation, [75.0, 30.0], [37.0, 8.0])
                + 2.8 * softbox(azimuth, elevation, [132.0, 57.0], [51.0, 27.0])
                + 1.45 * softbox(azimuth, elevation, [290.0, -43.0], [48.0, 23.0]);
            pixels.push(image::Rgb([radiance as f32; 3]));
        }
    }
    let mut encoded = Vec::new();
    image::codecs::hdr::HdrEncoder::new(&mut encoded).encode(&pixels, WIDTH, HEIGHT).map_err(|error| error.to_string())?;
    // The samples above are generated directly in linear sRGB.
    EnvironmentSource::hdr(encoded, 0.0, 1.0, true).map(Arc::new).map_err(|error| error.to_string())
}

fn softbox(azimuth: f64, elevation: f64, [heading, height]: [f64; 2], [half_width, half_height]: [f64; 2]) -> f64 {
    let horizontal = ((azimuth - heading.to_radians() + PI).rem_euclid(TAU) - PI).abs() / half_width.to_radians();
    let vertical = (elevation - height.to_radians()).abs() / half_height.to_radians();
    let edge = ((1.0 - horizontal.max(vertical)) / 0.35).clamp(0.0, 1.0);
    edge * edge * (3.0 - 2.0 * edge)
}

struct Frame {
    scene: ProductScene,
    pose: Pose,
    environment: Option<Arc<EnvironmentSource>>,
}

/// Trace the settled camera once while retaining the full preview resolution.
fn render(host: &ImageHost, serial: u64, revision: u64, frame: Frame, cancel: CancelToken, mut progress: impl FnMut(&'static str)) -> Outcome {
    let Frame { scene, pose, environment } = frame;
    let camera = pose.camera;
    let grazing = camera.back().z.abs() < 15_f64.to_radians().sin();
    let close_up = camera.distance < scene.bounds.diagonal() * 0.30;
    let [width, height] = image_size(pose.aspect, if grazing || close_up { 1440.0 } else { 960.0 });
    // Directional lights illuminate both sides without putting emitter geometry in the camera view.
    let directional_lights = DirectionalLights::new(&[
        DirectionalLightOptions { direction: [0.24, -0.4, 1.0], color: [1.0, 0.94, 0.86], intensity: 1.2 },
        DirectionalLightOptions { direction: [-0.24, -0.4, -1.0], color: [1.0, 0.94, 0.86], intensity: 1.2 },
    ])
    .map_err(|error| error.to_string())?;
    let options = JobOptions {
        samples: if grazing { 16 } else { 8 },
        max_depth: 3,
        lighting: Some(StudioLighting {
            environment: [if environment.is_some() { 0.0 } else { 0.55 }; 3],
            directional_lights,
            ..StudioLighting::default()
        }),
        denoise: true,
        ..JobOptions::default()
    };
    // The selected ImageHost supplies the compute integrator. The per-request CPU raytrace
    // override must stay empty when submitting to SolidCraft's GPU executor.
    let input = ImageRequest {
        camera,
        width,
        height,
        studio_weights: None,
        environment,
        textures: None,
        integrator: Default::default(),
        raytrace: None,
        background: [43, 46, 54],
        transparent_background: false,
    };
    cancel.check().map_err(|err| err.to_string())?;
    // PCBcraft revisions start at zero. SolidCraft reserves zero for invalid job identities.
    let generation = revision.saturating_add(1);
    let job_id = JobId { session: 1, serial, generation };
    // Carry navigation cancellation through CPU scene preparation as well as GPU execution.
    progress("Preparing ray-traced frame…");
    let prepared = host.prepare_submission(scene, input, options, None, cancel.clone()).map_err(|error| error.to_string())?;
    let job = host.submit_prepared(job_id, generation, prepared).map_err(|error| error.to_string())?;
    progress("Ray tracing…");
    loop {
        if cancel.is_cancelled() {
            job.cancel().map_err(|error| error.to_string())?;
        }
        let status = job.status().map_err(|error| error.to_string())?;
        if status.progress.phase.terminal() && job.work_released() {
            if status.progress.phase != Phase::Completed {
                return Err(status.error.map_or_else(|| "GPU ray tracing did not complete.".into(), |error| error.to_string()));
            }
            let frame = job.frame().map_err(|error| error.to_string())?.ok_or("The GPU completed without producing an image.")?;
            if frame.info.executor.as_str() != "gpu-path" {
                return Err("The renderer did not use the GPU path executor.".into());
            }
            let image = image::load_from_memory_with_format(frame.png(), image::ImageFormat::Png).map_err(|err| err.to_string())?.to_rgba8();
            return Ok(ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw()));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Declare green substrate and exposed conductor separately so the renderer resolves their
/// roughness and metallic reflection using its native material model.
fn material_mesh(material: Material, mesh: Mesh) -> Result<ProductMesh, String> {
    let (name, metallic, roughness) = match material {
        Material::Substrate => ("Board substrate", 0.0, 0.72),
        Material::Copper => ("Exterior copper", 1.0, 0.28),
    };
    let [r, g, b, _] = material.rgba();
    let [x, y, z] = [r, g, b].map(srgb8_to_linear);
    let surface = OpaqueSurface::new(Vec3::new(x, y, z), metallic, roughness, 1.48).map_err(|e| e.to_string())?;
    Ok(ProductMesh {
        name: name.into(),
        mesh: Arc::new(mesh),
        color: [r, g, b],
        opacity: 1.0,
        surface: Some(surface),
        transmission: None,
        faces: Vec::new(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ViewId;

    #[test]
    fn software_adapters_do_not_enable_ray_tracing() {
        assert!(!hardware(wgpu::DeviceType::Cpu));
        assert!(!hardware(wgpu::DeviceType::Other));
        assert!(hardware(wgpu::DeviceType::DiscreteGpu));
        assert!(!Worker::default().available());
    }

    #[test]
    fn cached_scene_requires_the_same_project_snapshot() {
        let mut session = pcbcraft_engine::Session::default();
        session.apply("project.new", &serde_json::json!({"create_board": true})).unwrap();
        let project = session.snapshot().unwrap();
        let copy = Arc::new((*project).clone());
        let request = Request { view: ViewId(0), target: project.board_target().unwrap(), revision: session.revision() };
        let entry = CachedScene { request, project: Arc::downgrade(&project), scene: ProductScene { bounds: Aabb3::EMPTY, meshes: Vec::new() } };
        assert!(entry.matches(request, &Arc::clone(&project)));
        assert!(!entry.matches(request, &copy));
        assert!(!entry.matches(Request { revision: request.revision + 1, ..request }, &project));
    }

    /// Exercise the real SolidCraft path tracer on a routed board, including an eight-degree view.
    #[test]
    #[ignore = "requires hardware Vulkan or Metal"]
    fn ray_traces_board_copper_and_substrate() {
        use solidcraft_viewport_gpu::select::{Policy, pick_path};
        let (adapter, report) = pick_path(Policy::Hardware, supports).unwrap();
        println!("Hardware ray tracing: {report:?}");
        let host = solidcraft_gpu::path::host_on_adapter(&adapter).unwrap();
        let path = std::env::var_os("PCBCRAFT_RAYTRACE_FIXTURE")
            .map_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sensor-node.pc"), std::path::PathBuf::from);
        println!("Board fixture: {}", path.display());
        let project = pcbcraft_storage::preview(&path).unwrap();
        let request = Request { view: ViewId(0), target: project.boards().next().unwrap().target, revision: 0 };
        let started = std::time::Instant::now();
        let scene = build_scene(&project, request, &CancelToken::default()).unwrap();
        println!("Board scene build: {:?}", started.elapsed());
        let mut camera = Camera::default();
        camera.fit_aspect(&solidcraft_viewport::vec::Bounds { min: scene.bounds.min, max: scene.bounds.max }, 1.5);
        let pose = Pose { camera, aspect: 1.5 };
        let overview = pose;
        camera.yaw = 4_f64.to_radians();
        camera.pitch = 8_f64.to_radians();
        camera.zoom_at(0.75, None);
        let grazing = Pose { camera, ..overview };
        let camera = Camera { distance: scene.bounds.diagonal() * 0.035, ..Camera::default() };
        let close_up = Pose { camera, ..overview };
        let mut coated = scene.clone();
        coat_board(&mut coated).unwrap();
        let studio = studio_environment().unwrap();
        let capture = std::env::var_os("PCBCRAFT_RAYTRACE_CAPTURE_DIR").is_some();
        let mut serial = 0;
        for (view, pose, size) in [("overview", overview, [960, 640]), ("grazing", grazing, [1440, 960]), ("close-up", close_up, [1440, 960])] {
            if view == "close-up" && !capture {
                continue;
            }
            let stages = [("current", &scene, None), ("studio", &scene, Some(&studio)), ("coated", &coated, Some(&studio))];
            let mut previous = None;
            for (look, scene, environment) in stages {
                if look != "studio" && !capture {
                    continue;
                }
                let started = std::time::Instant::now();
                serial += 1;
                let frame = Frame { scene: scene.clone(), pose, environment: environment.cloned() };
                let image = render(&host, serial, 0, frame, CancelToken::default(), |_| {}).unwrap();
                println!("{view} / {look}: {:?}", started.elapsed());
                assert_eq!(image.size, size);
                if view != "close-up" {
                    assert_has_copper_and_substrate(&image);
                }
                if let Some(previous) = previous {
                    assert_ne!(image.pixels, previous);
                }
                save_capture(&format!("{view}-{look}.png"), &image);
                previous = Some(image.pixels);
            }
        }

        let copper_z = |index: usize| {
            scene.meshes[index]
                .mesh
                .positions
                .iter()
                .map(|point| point.z)
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(bottom, top), z| (bottom.min(z), top.max(z)))
        };
        let (base_bottom, base_top) = copper_z(0_usize);
        let (copper_bottom, copper_top) = copper_z(1_usize);
        assert!(((copper_top - base_top) - super::super::copper::RAYTRACE_OFFSET_MM).abs() < 1e-9);
        assert!(((base_bottom - copper_bottom) - super::super::copper::RAYTRACE_OFFSET_MM).abs() < 1e-9);
    }

    /// Apply a thin clear coat over the green face material without changing the copper.
    fn coat_board(scene: &mut ProductScene) -> Result<(), String> {
        let board = scene.meshes.first_mut().ok_or("Missing board material.")?;
        let [r, g, b] = board.color.map(srgb8_to_linear);
        let coat = solidcraft_viewport::clear_coat::ClearCoat::new(0.65, 0.27, 1.49).map_err(|error| error.to_string())?;
        let body = OpaqueSurface::new(Vec3::new(r, g, b), 0.0, 0.92, 1.0).map_err(|error| error.to_string())?;
        board.surface = Some(body.with_clear_coat(coat));
        Ok(())
    }

    fn save_capture(name: &str, image: &ColorImage) {
        if let Some(directory) = std::env::var_os("PCBCRAFT_RAYTRACE_CAPTURE_DIR") {
            let path = std::path::PathBuf::from(directory).join(name);
            let bytes: Vec<_> = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
            image::save_buffer(path, &bytes, image.size[0] as u32, image.size[1] as u32, image::ColorType::Rgba8).unwrap();
        }
    }

    fn assert_has_copper_and_substrate(image: &ColorImage) {
        assert!(image.pixels.iter().filter(|pixel| pixel.g() > pixel.r().saturating_add(12)).count() > 40);
        assert!(image.pixels.iter().any(|pixel| pixel.r() > pixel.g().saturating_add(6)));
        let dark = image.pixels.iter().filter(|pixel| pixel.r() < 4 && pixel.g() < 4 && pixel.b() < 4).count();
        assert!(dark < image.pixels.len() / 100, "unexpected black emitter covering the view: {dark} pixels");
    }
}
