//! Bridge the disposable board mesh to SolidCraft's GPU renderer.
use std::sync::{Arc, Mutex};

use eframe::egui_wgpu::{self, RenderState, wgpu};
use egui::{Rect, Ui};
use pcbcraft_engine::board::view::BoardView;
use pcbcraft_geometry::substrate::BoardMesh;
use solidcore_record::Vec3;
use solidcraft_viewport::{
    Camera,
    vec::{Bounds, Vec3Ops},
};
use solidcraft_viewport_gpu::view::{Frame, GpuScene, GpuTarget, Renderer, SceneSlot};

pub(super) type Pipeline = Arc<Mutex<Renderer>>;

/// Build a renderer with its own offscreen depth buffer. Each pane has a separate picture and
/// camera, because the shared callback resource supports only one prepared frame per render pass.
pub(super) fn pipeline(rs: &RenderState) -> Pipeline {
    let window = GpuTarget { format: rs.target_format, depth: None, samples: 1, tess: false, oit: false };
    let scene = GpuTarget { depth: Some(wgpu::TextureFormat::Depth24Plus), ..window };
    Arc::new(Mutex::new(Renderer::with_window_target(&rs.device, scene, window, None)))
}

/// Prepare shaded triangles off-thread in millimeters. The scene is centered for GPU precision
/// while retaining the exact board/1 axis directions and relative distances. Triangle normals
/// keep the top and bottom faces flat and their transitions to the walls sharp.
pub(super) fn scene(mesh: BoardMesh, board: BoardView<'_>, thickness_nm: i64) -> Result<(Bounds, GpuScene), String> {
    let positions: Vec<_> = mesh.positions_nm.into_iter().map(|[x, y, z]| Vec3::new(x as f64, y as f64, z as f64) * 1e-6).collect();
    let mut bounds = Bounds::EMPTY;
    for &p in &positions {
        bounds.add(p);
    }
    let center = bounds.center();
    let bounds = Bounds { min: bounds.min - center, max: bounds.max - center };
    let mut gpu = GpuScene::default();
    let color = gpu.color([36, 134, 88, 255]);
    for triangle in mesh.triangles {
        let points = triangle.map(|index| positions[index as usize] - center);
        let [a, b, c] = points;
        if let Some(normal) = (b - a).cross(c - a).normalized() {
            for p in points {
                gpu.tri(vector(p), vector(normal), color);
            }
        }
    }
    if gpu.tris.is_empty() {
        return Err("The Board Outline produced no renderable faces.".into());
    }
    super::copper::append(&mut gpu, board, thickness_nm, center).map_err(|error| error.to_string())?;
    Ok((bounds, gpu))
}

/// Convert the shared camera/geometry vector to the GPU's vertex layout.
pub(super) fn vector(v: Vec3) -> [f32; 3] {
    [v.x as f32, v.y as f32, v.z as f32]
}

/// Pass a pane's camera and the current mesh to the shared offscreen renderer.
pub(super) fn paint(ui: &Ui, rect: Rect, camera: &Camera, bounds: &Bounds, key: u64, slot: &SceneSlot, pipeline: &Pipeline) {
    let projection = camera.view_proj_bounds(f64::from(rect.width() / rect.height()), bounds);
    let eye = vector(camera.eye());
    let empty = Arc::new(Mutex::new(None));
    let frame = Frame {
        product_lights: None,
        product: false,
        upload_error: None,
        key,
        slot: slot.clone(),
        sketch_key: 0,
        sketch_slot: empty.clone(),
        hl_key: 0,
        hl_slot: empty.clone(),
        pv_key: 0,
        pv_slot: empty,
        pv_hide: false,
        preview_bounds: None,
        view_proj: projection.to_f32(),
        turned_depth: None,
        back: vector(camera.back()),
        size_px: (rect.size() * ui.ctx().pixels_per_point()).into(),
        clip: [0.0; 4],
        cap: [0.0; 4],
        images: Vec::new(),
        clouds: Vec::new(),
        analysis: [0.0; 8],
        analysis_colors: [0.0; 36],
        access: None,
        outline: false,
        eye: [eye[0], eye[1], eye[2], 0.0],
        ao_radius: 0.0,
        ao_samples: 0,
        ssaa: 1.0,
        hover_tint: [0.0; 3],
        selection_tint: [0.0; 3],
        hatch: Default::default(),
        sky: [0.0; 4],
        moving: None,
    };
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, Callback { pipeline: pipeline.clone(), frame }));
}

struct Callback {
    pipeline: Pipeline,
    frame: Frame,
}

impl egui_wgpu::CallbackTrait for Callback {
    /// Prepare the offscreen picture belonging to this pane; sibling callbacks have separate renderers.
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Ok(mut renderer) = self.pipeline.lock() {
            renderer.prepare(device, queue, encoder, &self.frame);
        }
        Vec::new()
    }

    /// Composite this pane's picture in the egui pass at its DPI-adjusted rectangle.
    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, _resources: &egui_wgpu::CallbackResources) {
        let vp = info.viewport_in_pixels();
        if let Ok(renderer) = self.pipeline.lock() {
            renderer.paint(&self.frame, pass, [vp.left_px as f32, vp.top_px as f32, vp.width_px as f32, vp.height_px as f32]);
        }
    }
}
