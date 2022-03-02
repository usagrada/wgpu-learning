#![feature(once_cell)]
use futures::executor::LocalPool;
use futures::executor::LocalSpawner;
use wgpu::util::DeviceExt;
use wgpu::util::StagingBelt;
use wgpu_glyph::{
  ab_glyph::{self, FontArc},
  GlyphBrushBuilder, Section, Text,
};
use winit::{
  event::*,
  event_loop::{ControlFlow, EventLoop, EventLoopWindowTarget},
  window::Window,
};

static mut CNT: u32 = 0;

fn main() {
  env_logger::init();
  let event_loop = EventLoop::new();
  let window = Window::new(&event_loop).unwrap();
  window.set_title("todo app");

  event_loop.run(move |event, _window_event, control_flow| {
    event_handler(event, _window_event, control_flow, &window);
  });
}

fn event_handler(
  event: Event<()>,
  _window_event: &EventLoopWindowTarget<()>,
  control_flow: &mut ControlFlow,
  window: &Window,
) {
  *control_flow = ControlFlow::Wait;
  let mut state = pollster::block_on(State::new(&window));
  match event {
    Event::RedrawRequested(window_id) if window_id == window.id() => {
      state.update();
      match state.render(window.inner_size()) {
        Ok(_) => {}
        // Reconfigure the surface if lost
        Err(wgpu::SurfaceError::Lost) => state.resize(state.size),
        // The system is out of memory, we should probably quit
        Err(wgpu::SurfaceError::OutOfMemory) => *control_flow = ControlFlow::Exit,
        // All other errors (Outdated, Timeout) should be resolved by the next frame
        Err(e) => eprintln!("{:?}", e),
      }
    }
    Event::MainEventsCleared => {
      // RedrawRequested will only trigger once, unless we manually
      // request it.
      // window.request_redraw();
    }
    Event::WindowEvent { event, .. } => match event {
      WindowEvent::CloseRequested => {
        println!("The close button was pressed; stopping");
        *control_flow = ControlFlow::Exit;
      }
      WindowEvent::KeyboardInput { input, .. } => {
        if let Some(VirtualKeyCode::Escape) = input.virtual_keycode {
          *control_flow = ControlFlow::Exit;
        }
      }
      WindowEvent::Resized(size) => {
        state.resize(size);
        window.request_redraw();
      }
      WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
        state.resize(window.inner_size());
      }
      _ => {}
    },
    _ => {
      println!("{:?}", event);
    }
  }
  unsafe {
    if (CNT & 0b1111) == 0 {
      println!("Hello, world! count:{CNT}");
    }
    CNT += 1;
  }
}

struct State {
  surface: wgpu::Surface,
  device: wgpu::Device,
  queue: wgpu::Queue,
  config: wgpu::SurfaceConfiguration,
  size: winit::dpi::PhysicalSize<u32>,
  // NEW!
  render_pipeline: wgpu::RenderPipeline,
  glyph_brush: wgpu_glyph::GlyphBrush<()>,
  staging_belt: StagingBelt,
  // NEW!
  vertex_buffer: wgpu::Buffer,
  num_vertices: u32,
  local_pool: LocalPool,
  local_spawner: LocalSpawner,
}

impl State {
  // Creating some of the wgpu types requires async code
  async fn new(window: &Window) -> Self {
    let size = window.inner_size();

    // The instance is a handle to our GPU
    // Backends::all => Vulkan + Metal + DX12 + Browser WebGPU
    let instance = wgpu::Instance::new(wgpu::Backends::all());
    let surface = unsafe { instance.create_surface(window) };
    let adapter = instance
      .request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
      })
      .await
      .unwrap();

    let (device, queue) = adapter
      .request_device(
        &wgpu::DeviceDescriptor {
          features: wgpu::Features::empty(),
          limits: wgpu::Limits::default(),
          label: None,
        },
        None, // Trace path
      )
      .await
      .unwrap();

    let config = wgpu::SurfaceConfiguration {
      usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
      format: surface.get_preferred_format(&adapter).unwrap(),
      width: size.width,
      height: size.height,
      present_mode: wgpu::PresentMode::Fifo,
    };
    surface.configure(&device, &config);

    let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
      label: Some("Shader"),
      source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
    });

    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
      label: Some("Render Pipeline Layout"),
      bind_group_layouts: &[],
      push_constant_ranges: &[],
    });

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
      label: Some("Render Pipeline"),
      layout: Some(&render_pipeline_layout),
      vertex: wgpu::VertexState {
        module: &shader,
        entry_point: "vs_main", // 1.
        // buffers: &[],           // 2.
        buffers: &[Vertex::desc()], // 3
      },
      fragment: Some(wgpu::FragmentState {
        // 3.
        module: &shader,
        entry_point: "fs_main",
        targets: &[wgpu::ColorTargetState {
          // 4.
          format: config.format,
          blend: Some(wgpu::BlendState::REPLACE),
          write_mask: wgpu::ColorWrites::ALL,
        }],
      }),
      primitive: wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleList, // 1.
        strip_index_format: None,
        front_face: wgpu::FrontFace::Ccw, // 2.
        cull_mode: Some(wgpu::Face::Back),
        // Setting this to anything other than Fill requires Features::NON_FILL_POLYGON_MODE
        polygon_mode: wgpu::PolygonMode::Fill,
        // Requires Features::DEPTH_CLIP_CONTROL
        unclipped_depth: false,
        // Requires Features::CONSERVATIVE_RASTERIZATION
        conservative: false,
      },
      depth_stencil: None, // 1.
      multisample: wgpu::MultisampleState {
        count: 1,                         // 2.
        mask: !0,                         // 3.
        alpha_to_coverage_enabled: false, // 4.
      },
      multiview: None, // 5.
    });

    let font: FontArc =
      ab_glyph::FontArc::try_from_slice(include_bytes!("ipag.ttf")).unwrap();
    let glyph_brush = GlyphBrushBuilder::using_font(font).build(&device, config.format);
    // Create staging belt and a local pool
    let staging_belt = wgpu::util::StagingBelt::new(4096);

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
      label: Some("Vertex Buffer"),
      contents: bytemuck::cast_slice(VERTICES),
      usage: wgpu::BufferUsages::VERTEX,
    });
    let num_vertices = VERTICES.len() as u32;
    let local_pool = futures::executor::LocalPool::new();
    let local_spawner = local_pool.spawner();

    Self {
      surface,
      device,
      queue,
      config,
      size,
      render_pipeline,
      glyph_brush,
      staging_belt,
      vertex_buffer,
      num_vertices,
      local_pool,
      local_spawner,
    }
  }

  pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
    if new_size.width > 0 && new_size.height > 0 {
      self.size = new_size;
      self.config.width = new_size.width;
      self.config.height = new_size.height;
      self.surface.configure(&self.device, &self.config);
    }
  }

  fn input(&mut self, event: &WindowEvent) -> bool {
    false
  }

  fn update(&mut self) {
    println!("update");
  }

  fn render(&mut self, size: winit::dpi::PhysicalSize<u32>) -> Result<(), wgpu::SurfaceError> {
    let output = self.surface.get_current_texture()?;
    let view = output
      .texture
      .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Render Encoder"),
      });

    {
      let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Render Pass"),
        color_attachments: &[wgpu::RenderPassColorAttachment {
          view: &view,
          resolve_target: None,
          ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color {
              r: 0.1,
              g: 0.2,
              b: 0.3,
              a: 1.0,
            }),
            store: true,
          },
        }],
        depth_stencil_attachment: None,
      });
      render_pass.set_pipeline(&self.render_pipeline); // 2.
      render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
      // render_pass.draw(0..3, 0..1); // 3.
      render_pass.draw(0..self.num_vertices, 0..1);
    }

    // Draw the text!
    self.glyph_brush.queue(Section {
      screen_position: (30.0, 30.0),
      bounds: (size.width as f32, size.height as f32),
      text: vec![Text::new("Hello wgpu_glyph 日本語だよ!")
        .with_color([0.0, 0.0, 0.0, 1.0])
        .with_scale(40.0)],
      ..Section::default()
    });

    self.glyph_brush.queue(Section {
      screen_position: (30.0, 90.0),
      bounds: (size.width as f32, size.height as f32),
      text: vec![Text::new("Hello wgpu_glyph!")
        .with_color([1.0, 1.0, 1.0, 1.0])
        .with_scale(40.0)],
      ..Section::default()
    });
    self
      .glyph_brush
      .draw_queued(
        &self.device,
        &mut self.staging_belt,
        &mut encoder,
        &view,
        size.width,
        size.height,
      )
      .expect("Draw queued");

    // submit will accept anything that implements IntoIter
    self.staging_belt.finish();
    self.queue.submit(std::iter::once(encoder.finish()));
    output.present();

    // Recall unused staging buffers
    use futures::task::SpawnExt;
    self.local_spawner
        .spawn(self.staging_belt.recall())
        .expect("Recall staging belt");

    self.local_pool.run_until_stalled();
    Ok(())
  }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
  position: [f32; 3],
  color: [f32; 3],
}

const VERTICES: &[Vertex] = &[
  Vertex {
    position: [0.0, 0.5, 0.0],
    color: [1.0, 0.0, 0.0],
  },
  Vertex {
    position: [-0.5, -0.5, 0.0],
    color: [0.0, 1.0, 0.0],
  },
  Vertex {
    position: [0.5, -0.5, 0.0],
    color: [0.0, 0.0, 1.0],
  },
];

impl Vertex {
  fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
    wgpu::VertexBufferLayout {
      array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
      step_mode: wgpu::VertexStepMode::Vertex,
      attributes: &[
        wgpu::VertexAttribute {
          offset: 0,
          shader_location: 0,
          format: wgpu::VertexFormat::Float32x3,
        },
        wgpu::VertexAttribute {
          offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
          shader_location: 1,
          format: wgpu::VertexFormat::Float32x3,
        },
      ],
    }
  }
}
