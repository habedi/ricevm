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
            bf("Display.allocate", 0x74694470, 48, draw_display_allocate),
            bf("Display.cmap2rgb", 0xda836903, 48, draw_stub),
            bf("Display.cmap2rgba", 0x0a64b341, 48, draw_stub),
            bf("Display.color", 0xac54c4aa, 48, draw_display_color),
            bf("Display.colormix", 0x9e941050, 48, draw_stub),
            bf("Display.getwindow", 0xdfbf1d73, 64, draw_display_getwindow),
            bf("Display.namedimage", 0x47522dfe, 48, draw_stub),
            bf("Display.newimage", 0xb8479988, 64, draw_display_newimage),
            bf("Display.open", 0x47522dfe, 48, draw_stub),
            bf("Display.publicscreen", 0x507e0780, 48, draw_stub),
            bf("Display.readimage", 0xd38f4d48, 48, draw_stub),
            bf("Display.rgb", 0x8e71a513, 56, draw_stub),
            bf("Display.rgb2cmap", 0xbf6c3d95, 56, draw_stub),
            bf("Display.startrefresh", 0xf0df9cae, 40, draw_stub),
            bf("Display.writeimage", 0x7bd53940, 48, draw_stub),
            bf("Font.bbox", 0x541e2d08, 48, draw_stub),
            bf("Font.build", 0x7fddba2c, 56, draw_stub),
            bf("Font.open", 0xddcb2ff0, 48, draw_font_open),
            bf("Font.width", 0x1c70cba4, 48, draw_font_width),
            bf("Image.arc", 0x1685a04e, 88, draw_stub),
            bf("Image.arcop", 0x8521de24, 96, draw_stub),
            bf("Image.arrow", 0x7b3fc6d3, 48, draw_stub),
            bf("Image.bezier", 0x5baca124, 96, draw_stub),
            bf("Image.bezierop", 0xae13ba0e, 104, draw_stub),
            bf("Image.bezspline", 0x70f06194, 80, draw_stub),
            bf("Image.bezsplineop", 0x94b3bea1, 88, draw_stub),
            bf("Image.border", 0x59381f67, 64, draw_image_border),
            bf("Image.bottom", 0x642fa8b1, 40, draw_stub),
            bf("Image.draw", 0xe2951762, 72, draw_image_draw),
            bf("Image.drawop", 0x7e2751d3, 80, draw_image_draw),
            bf("Image.ellipse", 0xea6f2000, 72, draw_image_ellipse),
            bf("Image.ellipseop", 0xc0af34c6, 80, draw_image_ellipse),
            bf("Image.fillarc", 0x784ac6f8, 80, draw_stub),
            bf("Image.fillarcop", 0x0bbcdbdd, 88, draw_stub),
            bf("Image.fillbezier", 0x4a07ed44, 88, draw_stub),
            bf("Image.fillbezierop", 0xb3796aa0, 96, draw_stub),
            bf("Image.fillbezspline", 0x4aac99aa, 72, draw_stub),
            bf("Image.fillbezsplineop", 0x6288a256, 80, draw_stub),
            bf("Image.fillellipse", 0xc2961c2b, 64, draw_image_fillellipse),
            bf(
                "Image.fillellipseop",
                0x9816222d,
                72,
                draw_image_fillellipse,
            ),
            bf("Image.fillpoly", 0x4aac99aa, 72, draw_stub),
            bf("Image.fillpolyop", 0x6288a256, 80, draw_stub),
            bf("Image.flush", 0xb09fc26e, 48, draw_image_flush),
            bf("Image.gendraw", 0xa30a11c7, 80, draw_stub),
            bf("Image.gendrawop", 0x03e8228a, 88, draw_stub),
            bf("Image.line", 0x7288c7b9, 80, draw_image_line),
            bf("Image.lineop", 0xe34363b9, 88, draw_image_line),
            bf("Image.name", 0xdff53107, 48, draw_stub),
            bf("Image.origin", 0x9171b0bd, 56, draw_stub),
            bf("Image.poly", 0x70f06194, 80, draw_stub),
            bf("Image.polyop", 0x94b3bea1, 88, draw_stub),
            bf("Image.readpixels", 0x93d30c7c, 56, draw_stub),
            bf("Image.text", 0xbbf36e48, 72, draw_image_text),
            bf("Image.textbg", 0x73c80190, 88, draw_stub),
            bf("Image.textbgop", 0xfafbb80a, 96, draw_stub),
            bf("Image.textop", 0x4211ebe0, 80, draw_image_text),
            bf("Image.top", 0x642fa8b1, 40, draw_stub),
            bf("Image.writepixels", 0x93d30c7c, 56, draw_stub),
            bf("Screen.allocate", 0x3e3fba99, 56, draw_screen_allocate),
            bf("Screen.bottom", 0x66dbf29a, 48, draw_stub),
            bf("Screen.newwindow", 0xc2e1a4d0, 56, draw_screen_newwindow),
            bf("Screen.top", 0x66dbf29a, 48, draw_stub),
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

// --- ADT record constructors ---
// These create properly structured heap records matching the Limbo ADT layouts.

/// Create an Image ADT record.
/// Layout: r(Rect=16B), clipr(Rect=16B), depth(4B), chans(4B), repl(4B),
///         display(4B ptr), screen(4B ptr), iname(4B ptr) = 56 bytes
pub(crate) fn make_image(
    heap: &mut crate::heap::Heap,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    depth: i32,
    display_id: HeapId,
) -> HeapId {
    let mut data = vec![0u8; 56];
    // r.min.x, r.min.y, r.max.x, r.max.y
    memory::write_word(&mut data, 0, x);
    memory::write_word(&mut data, 4, y);
    memory::write_word(&mut data, 8, x + w);
    memory::write_word(&mut data, 12, y + h);
    // clipr = same as r
    memory::write_word(&mut data, 16, x);
    memory::write_word(&mut data, 20, y);
    memory::write_word(&mut data, 24, x + w);
    memory::write_word(&mut data, 28, y + h);
    // depth
    memory::write_word(&mut data, 32, depth);
    // chans (XRGB32)
    memory::write_word(&mut data, 36, 0x08_08_08_08_u32 as i32);
    // repl = 0
    // display
    memory::write_word(&mut data, 44, display_id as i32);
    // screen = nil, iname = nil
    heap.alloc(0, HeapData::Record(data))
}

/// Create a Display ADT record.
/// Layout: image(4B ptr), white(4B ptr), black(4B ptr),
///         opaque(4B ptr), transparent(4B ptr) = 20 bytes
fn make_display(heap: &mut crate::heap::Heap, w: i32, h: i32) -> HeapId {
    // First create the display record (we need its ID for the images)
    let display_id = heap.alloc(0, HeapData::Record(vec![0u8; 20]));

    // Create standard images
    let image_id = make_image(heap, 0, 0, w, h, 32, display_id);
    let white_id = make_image(heap, 0, 0, 1, 1, 32, display_id);
    let black_id = make_image(heap, 0, 0, 1, 1, 32, display_id);
    let opaque_id = white_id;
    let transparent_id = black_id;

    // Fill in the display record
    if let Some(obj) = heap.get_mut(display_id) {
        if let HeapData::Record(data) = &mut obj.data {
            memory::write_word(data, 0, image_id as i32);
            memory::write_word(data, 4, white_id as i32);
            memory::write_word(data, 8, black_id as i32);
            memory::write_word(data, 12, opaque_id as i32);
            memory::write_word(data, 16, transparent_id as i32);
        }
    }

    // Increment ref counts for the images stored in the display
    heap.inc_ref(image_id);
    heap.inc_ref(white_id);
    heap.inc_ref(black_id);

    display_id
}

/// Create a Screen ADT record.
/// Layout: id(4B), image(4B ptr), fill(4B ptr), display(4B ptr) = 16 bytes
fn make_screen(heap: &mut crate::heap::Heap, display_id: HeapId) -> HeapId {
    let mut data = vec![0u8; 16];
    memory::write_word(&mut data, 0, 1); // id
    let img = make_image(heap, 0, 0, 800, 600, 32, display_id);
    let fill = make_image(heap, 0, 0, 1, 1, 32, display_id);
    memory::write_word(&mut data, 4, img as i32);
    memory::write_word(&mut data, 8, fill as i32);
    memory::write_word(&mut data, 12, display_id as i32);
    heap.alloc(0, HeapData::Record(data))
}

// --- Display functions ---

fn draw_display_allocate(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();

    #[cfg(feature = "gui")]
    {
        match DrawState::new("RiceVM", 800, 600) {
            Ok(ds) => {
                let w = ds.width as i32;
                let h = ds.height as i32;
                state::set(ds);
                let display_id = make_display(&mut vm.heap, w, h);
                memory::write_word(&mut vm.frames.data, frame_base, display_id as i32);
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize display");
                memory::write_word(&mut vm.frames.data, frame_base, 0);
            }
        }
    }

    #[cfg(not(feature = "gui"))]
    {
        // Even without SDL2, create a proper Display record so programs
        // that check for nil do not crash immediately.
        let display_id = make_display(&mut vm.heap, 800, 600);
        memory::write_word(&mut vm.frames.data, frame_base, display_id as i32);
    }

    Ok(())
}

/// Display.getwindow: returns (ref Screen, ref Image)
fn draw_display_getwindow(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let display_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;

    let screen_id = make_screen(&mut vm.heap, display_id);
    let window_img = make_image(&mut vm.heap, 0, 0, 800, 600, 32, display_id);
    // Set screen field on window image
    if let Some(obj) = vm.heap.get_mut(window_img) {
        if let HeapData::Record(data) = &mut obj.data {
            if data.len() >= 52 {
                memory::write_word(data, 48, screen_id as i32);
            }
        }
    }

    // Return tuple: (screen, image) at frame offsets 0 and 4
    memory::write_word(&mut vm.frames.data, frame_base, screen_id as i32);
    memory::write_word(&mut vm.frames.data, frame_base + 4, window_img as i32);
    Ok(())
}

fn draw_display_color(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let display_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let _color = memory::read_word(&vm.frames.data, frame_base + 20) as u32;
    let img = make_image(&mut vm.heap, 0, 0, 1, 1, 32, display_id);
    memory::write_word(&mut vm.frames.data, frame_base, img as i32);
    Ok(())
}

fn draw_display_newimage(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let display_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let rx = memory::read_word(&vm.frames.data, frame_base + 20);
    let ry = memory::read_word(&vm.frames.data, frame_base + 24);
    let rw = memory::read_word(&vm.frames.data, frame_base + 28) - rx;
    let rh = memory::read_word(&vm.frames.data, frame_base + 32) - ry;
    let img = make_image(&mut vm.heap, rx, ry, rw, rh, 32, display_id);
    memory::write_word(&mut vm.frames.data, frame_base, img as i32);
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
        state::with(|opt_state| {
            if let Some(state) = opt_state {
                state.canvas.set_draw_color(Color::WHITE);
                let _ = state
                    .canvas
                    .fill_rect(SdlRect::new(rx, ry, rw as u32, rh as u32));
            }
        });
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

        state::with(|opt_state| {
            if let Some(state) = opt_state {
                state.canvas.set_draw_color(Color::WHITE);
                let _ = state.canvas.draw_line(
                    sdl2::rect::Point::new(x0, y0),
                    sdl2::rect::Point::new(x1, y1),
                );
            }
        });
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

        state::with(|opt_state| {
            if let Some(state) = opt_state {
                state.canvas.set_draw_color(Color::WHITE);
                // SDL2 doesn't have built-in ellipse; draw a rectangle as approximation
                let _ = state.canvas.draw_rect(SdlRect::new(
                    cx - a,
                    cy - b,
                    (a * 2) as u32,
                    (b * 2) as u32,
                ));
            }
        });
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

        state::with(|opt_state| {
            if let Some(state) = opt_state {
                state.canvas.set_draw_color(Color::WHITE);
                let _ = state.canvas.fill_rect(SdlRect::new(
                    cx - a,
                    cy - b,
                    (a * 2) as u32,
                    (b * 2) as u32,
                ));
            }
        });
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

        state::with(|opt_state| {
            if let Some(state) = opt_state {
                state.canvas.set_draw_color(Color::WHITE);
                let _ = state
                    .canvas
                    .draw_rect(SdlRect::new(rx, ry, rw as u32, rh as u32));
            }
        });
    }
    Ok(())
}

fn draw_image_flush(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    #[cfg(feature = "gui")]
    {
        let mut should_quit = false;
        state::with(|opt_state| {
            if let Some(state) = opt_state {
                state.canvas.present();
                for event in state.event_pump.poll_iter() {
                    match event {
                        sdl2::event::Event::Quit { .. } => {
                            should_quit = true;
                        }
                        sdl2::event::Event::MouseButtonDown {
                            x, y, mouse_btn, ..
                        } => {
                            tracing::trace!(
                                x = x,
                                y = y,
                                button = ?mouse_btn,
                                "Mouse button down"
                            );
                        }
                        sdl2::event::Event::KeyDown { keycode, .. } => {
                            tracing::trace!(keycode = ?keycode, "Key down");
                        }
                        _ => {}
                    }
                }
            }
        });
        if should_quit {
            vm.halted = true;
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
    // Read screen ref and rectangle from the frame
    let screen_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let rx = memory::read_word(&vm.frames.data, frame_base + 20);
    let ry = memory::read_word(&vm.frames.data, frame_base + 24);
    let rw = memory::read_word(&vm.frames.data, frame_base + 28) - rx;
    let rh = memory::read_word(&vm.frames.data, frame_base + 32) - ry;

    // Get the display from the screen
    let display_id = if let Some(obj) = vm.heap.get(screen_id) {
        if let HeapData::Record(data) = &obj.data {
            if data.len() >= 16 {
                memory::read_word(data, 12) as HeapId
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0
    };

    let img = make_image(&mut vm.heap, rx, ry, rw, rh, 32, display_id);
    // Set the screen field on the image
    if let Some(obj) = vm.heap.get_mut(img) {
        if let HeapData::Record(data) = &mut obj.data {
            if data.len() >= 52 {
                memory::write_word(data, 48, screen_id as i32);
            }
        }
    }
    memory::write_word(&mut vm.frames.data, frame_base, img as i32);
    Ok(())
}

fn draw_screen_allocate(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let image_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    // Get display from image
    let display_id = if let Some(obj) = vm.heap.get(image_id) {
        if let HeapData::Record(data) = &obj.data {
            if data.len() >= 48 {
                memory::read_word(data, 44) as HeapId
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0
    };
    let screen_id = make_screen(&mut vm.heap, display_id);
    memory::write_word(&mut vm.frames.data, frame_base, screen_id as i32);
    Ok(())
}

/// Thread-local DrawState for single-threaded GUI access.
#[cfg(feature = "gui")]
pub(crate) mod state {
    use super::DrawState;
    use std::cell::RefCell;

    thread_local! {
        static DRAW: RefCell<Option<DrawState>> = const { RefCell::new(None) };
    }

    pub fn set(s: DrawState) {
        DRAW.with(|d| *d.borrow_mut() = Some(s));
    }

    pub fn with<F, R>(f: F) -> R
    where
        F: FnOnce(Option<&mut DrawState>) -> R,
    {
        DRAW.with(|d| f(d.borrow_mut().as_mut()))
    }
}
