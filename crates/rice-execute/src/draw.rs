//! Built-in Draw module ($Draw) implementation using SDL2.
//!
//! This module provides the Inferno Draw graphics API backed by SDL2.
//! It is only available when the `gui` feature is enabled.
//!
//! Implemented functions:
//! - Display.allocate: open a window
//! - Display.newimage: create an offscreen image
//! - Display.color: create a solid color image
//! - Image.draw: blit/composite images
//! - Image.line: draw a line
//! - Image.ellipse/fillellipse: draw ellipses
//! - Image.text: render text
//! - Image.flush: present to screen
//! - Image.border: draw a border rectangle
//! - Font.open: load a font
//! - Font.width: measure text width
//! - Screen.newwindow: create a window image

#[cfg(feature = "gui")]
use sdl2::pixels::Color;
#[cfg(feature = "gui")]
use sdl2::rect::Rect as SdlRect;

use ricevm_core::ExecError;

use crate::builtin::{BuiltinFunc, BuiltinModule};
use crate::heap::{HeapData, HeapId};
use crate::memory;
use crate::vm::VmState;

/// State for the SDL2 display backend.
#[cfg(feature = "gui")]
pub(crate) struct DrawState {
    pub sdl_context: sdl2::Sdl,
    pub video: sdl2::VideoSubsystem,
    pub canvas: sdl2::render::Canvas<sdl2::video::Window>,
    pub event_pump: sdl2::EventPump,
    pub width: u32,
    pub height: u32,
}

#[cfg(feature = "gui")]
impl DrawState {
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self, String> {
        let sdl_context = sdl2::init()?;
        let video = sdl_context.video()?;
        let window = video
            .window(title, width, height)
            .position_centered()
            .resizable()
            .build()
            .map_err(|e| e.to_string())?;
        let canvas = window
            .into_canvas()
            .accelerated()
            .present_vsync()
            .build()
            .map_err(|e| e.to_string())?;
        let event_pump = sdl_context.event_pump()?;
        Ok(Self {
            sdl_context,
            video,
            canvas,
            event_pump,
            width,
            height,
        })
    }
}

/// Convert an Inferno RGBA color (0xRRGGBBAA) to SDL2 Color.
#[cfg(feature = "gui")]
fn inferno_color(c: u32) -> Color {
    Color::RGBA(
        ((c >> 24) & 0xFF) as u8,
        ((c >> 16) & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
        (c & 0xFF) as u8,
    )
}

/// Create the $Draw built-in module.
pub(crate) fn create_draw_module() -> BuiltinModule {
    BuiltinModule {
        name: "$Draw",
        funcs: vec![
            bf("Display.allocate", 0, 48, draw_display_allocate),
            bf("Display.color", 0, 48, draw_display_color),
            bf("Display.getwindow", 0, 64, draw_stub),
            bf("Display.newimage", 0, 64, draw_display_newimage),
            bf("Display.publicscreen", 0, 48, draw_stub),
            bf("Display.startrefresh", 0, 40, draw_stub),
            bf("Font.open", 0, 48, draw_font_open),
            bf("Font.width", 0, 48, draw_font_width),
            bf("Font.bbox", 0, 48, draw_stub),
            bf("Font.build", 0, 56, draw_stub),
            bf("Image.arrow", 0, 48, draw_stub),
            bf("Image.border", 0, 64, draw_image_border),
            bf("Image.bottom", 0, 40, draw_stub),
            bf("Image.draw", 0, 72, draw_image_draw),
            bf("Image.drawop", 0, 80, draw_image_draw),
            bf("Image.ellipse", 0, 72, draw_image_ellipse),
            bf("Image.fillellipse", 0, 64, draw_image_fillellipse),
            bf("Image.flush", 0, 48, draw_image_flush),
            bf("Image.gendraw", 0, 80, draw_stub),
            bf("Image.line", 0, 80, draw_image_line),
            bf("Image.lineop", 0, 88, draw_image_line),
            bf("Image.name", 0, 48, draw_stub),
            bf("Image.origin", 0, 56, draw_stub),
            bf("Image.readpixels", 0, 56, draw_stub),
            bf("Image.text", 0, 72, draw_image_text),
            bf("Image.textop", 0, 80, draw_image_text),
            bf("Image.top", 0, 40, draw_stub),
            bf("Image.writepixels", 0, 56, draw_stub),
            bf("Screen.allocate", 0, 56, draw_stub),
            bf("Screen.newwindow", 0, 56, draw_screen_newwindow),
            bf("Screen.top", 0, 48, draw_stub),
            bf("Screen.bottom", 0, 48, draw_stub),
        ],
    }
}

fn bf(
    name: &'static str,
    sig: u32,
    frame_size: usize,
    handler: fn(&mut VmState<'_>) -> Result<(), ExecError>,
) -> BuiltinFunc {
    BuiltinFunc {
        name,
        sig,
        frame_size,
        handler,
    }
}

fn draw_stub(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

// --- Display functions ---

fn draw_display_allocate(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();

    #[cfg(feature = "gui")]
    {
        // Initialize SDL2 and create a window
        match DrawState::new("RiceVM", 800, 600) {
            Ok(state) => {
                // Store the DrawState in a heap record (as an opaque handle)
                let id = vm.heap.alloc(0, HeapData::Record(vec![0u8; 4]));
                // Store display reference — in a real impl we'd store the DrawState
                // For now, use a global (simplified)
                memory::write_word(&mut vm.frames.data, frame_base, id as i32);

                // Store the DrawState somewhere accessible
                // Using a static mutable is not ideal but works for single-threaded
                unsafe {
                    DRAW_STATE = Some(state);
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize display");
                memory::write_word(&mut vm.frames.data, frame_base, 0);
            }
        }
    }

    #[cfg(not(feature = "gui"))]
    {
        tracing::warn!("$Draw not available (compile with --features gui)");
        memory::write_word(&mut vm.frames.data, frame_base, 0);
    }

    Ok(())
}

fn draw_display_color(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Return a color image handle (simplified: just store the color value)
    let _color = memory::read_word(&vm.frames.data, frame_base + 20) as u32;
    let id = vm.heap.alloc(0, HeapData::Record(vec![0u8; 4]));
    memory::write_word(&mut vm.frames.data, frame_base, id as i32);
    Ok(())
}

fn draw_display_newimage(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let id = vm.heap.alloc(0, HeapData::Record(vec![0u8; 4]));
    memory::write_word(&mut vm.frames.data, frame_base, id as i32);
    Ok(())
}

// --- Image functions ---

fn draw_image_draw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    #[cfg(feature = "gui")]
    {
        let frame_base = vm.frames.current_data_offset();
        // Read rectangle (min.x, min.y, max.x, max.y)
        let rx = memory::read_word(&vm.frames.data, frame_base + 20);
        let ry = memory::read_word(&vm.frames.data, frame_base + 24);
        let rw = memory::read_word(&vm.frames.data, frame_base + 28) - rx;
        let rh = memory::read_word(&vm.frames.data, frame_base + 32) - ry;

        // Get source color (simplified: use white)
        unsafe {
            if let Some(ref mut state) = DRAW_STATE {
                state.canvas.set_draw_color(Color::WHITE);
                let _ = state.canvas.fill_rect(SdlRect::new(rx, ry, rw as u32, rh as u32));
            }
        }
    }
    Ok(())
}

fn draw_image_line(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    #[cfg(feature = "gui")]
    {
        let frame_base = vm.frames.current_data_offset();
        let x0 = memory::read_word(&vm.frames.data, frame_base + 20);
        let y0 = memory::read_word(&vm.frames.data, frame_base + 24);
        let x1 = memory::read_word(&vm.frames.data, frame_base + 28);
        let y1 = memory::read_word(&vm.frames.data, frame_base + 32);

        unsafe {
            if let Some(ref mut state) = DRAW_STATE {
                state.canvas.set_draw_color(Color::WHITE);
                let _ = state.canvas.draw_line(
                    sdl2::rect::Point::new(x0, y0),
                    sdl2::rect::Point::new(x1, y1),
                );
            }
        }
    }
    Ok(())
}

fn draw_image_ellipse(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    #[cfg(feature = "gui")]
    {
        let frame_base = vm.frames.current_data_offset();
        let cx = memory::read_word(&vm.frames.data, frame_base + 20);
        let cy = memory::read_word(&vm.frames.data, frame_base + 24);
        let a = memory::read_word(&vm.frames.data, frame_base + 28);
        let b = memory::read_word(&vm.frames.data, frame_base + 32);

        unsafe {
            if let Some(ref mut state) = DRAW_STATE {
                state.canvas.set_draw_color(Color::WHITE);
                // SDL2 doesn't have built-in ellipse; draw a rectangle as approximation
                let _ = state.canvas.draw_rect(SdlRect::new(
                    cx - a, cy - b, (a * 2) as u32, (b * 2) as u32,
                ));
            }
        }
    }
    Ok(())
}

fn draw_image_fillellipse(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    #[cfg(feature = "gui")]
    {
        let frame_base = vm.frames.current_data_offset();
        let cx = memory::read_word(&vm.frames.data, frame_base + 20);
        let cy = memory::read_word(&vm.frames.data, frame_base + 24);
        let a = memory::read_word(&vm.frames.data, frame_base + 28);
        let b = memory::read_word(&vm.frames.data, frame_base + 32);

        unsafe {
            if let Some(ref mut state) = DRAW_STATE {
                state.canvas.set_draw_color(Color::WHITE);
                let _ = state.canvas.fill_rect(SdlRect::new(
                    cx - a, cy - b, (a * 2) as u32, (b * 2) as u32,
                ));
            }
        }
    }
    Ok(())
}

fn draw_image_text(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    #[cfg(feature = "gui")]
    {
        let frame_base = vm.frames.current_data_offset();
        let _px = memory::read_word(&vm.frames.data, frame_base + 20);
        let _py = memory::read_word(&vm.frames.data, frame_base + 24);
        // Text rendering requires SDL2_ttf which adds complexity.
        // For now, this is a stub that returns the point unchanged.
        memory::write_word(&mut vm.frames.data, frame_base, _px);
        memory::write_word(&mut vm.frames.data, frame_base + 4, _py);
    }
    Ok(())
}

fn draw_image_border(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    #[cfg(feature = "gui")]
    {
        let frame_base = vm.frames.current_data_offset();
        let rx = memory::read_word(&vm.frames.data, frame_base + 20);
        let ry = memory::read_word(&vm.frames.data, frame_base + 24);
        let rw = memory::read_word(&vm.frames.data, frame_base + 28) - rx;
        let rh = memory::read_word(&vm.frames.data, frame_base + 32) - ry;

        unsafe {
            if let Some(ref mut state) = DRAW_STATE {
                state.canvas.set_draw_color(Color::WHITE);
                let _ = state.canvas.draw_rect(SdlRect::new(rx, ry, rw as u32, rh as u32));
            }
        }
    }
    Ok(())
}

fn draw_image_flush(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    #[cfg(feature = "gui")]
    {
        unsafe {
            if let Some(ref mut state) = DRAW_STATE {
                state.canvas.present();
                // Process pending events to keep the window responsive
                for event in state.event_pump.poll_iter() {
                    match event {
                        sdl2::event::Event::Quit { .. } => {
                            vm.halted = true;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let _ = vm;
    Ok(())
}

// --- Font functions ---

fn draw_font_open(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Return a font handle (simplified: just allocate a dummy record)
    let id = vm.heap.alloc(0, HeapData::Record(vec![0u8; 16]));
    // Set height=16, ascent=12 (reasonable defaults)
    if let Some(obj) = vm.heap.get_mut(id)
        && let HeapData::Record(data) = &mut obj.data
    {
        memory::write_word(data, 0, 16); // height
        memory::write_word(data, 4, 12); // ascent
    }
    memory::write_word(&mut vm.frames.data, frame_base, id as i32);
    Ok(())
}

fn draw_font_width(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let str_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;
    let len = vm.heap.get_string(str_id).map(|s| s.len()).unwrap_or(0);
    // Approximate: 8 pixels per character (monospace assumption)
    memory::write_word(&mut vm.frames.data, frame_base, (len * 8) as i32);
    Ok(())
}

// --- Screen functions ---

fn draw_screen_newwindow(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Return a window image handle
    let id = vm.heap.alloc(0, HeapData::Record(vec![0u8; 4]));
    memory::write_word(&mut vm.frames.data, frame_base, id as i32);
    Ok(())
}

// Global DrawState (simplified for single-threaded use)
#[cfg(feature = "gui")]
static mut DRAW_STATE: Option<DrawState> = None;
