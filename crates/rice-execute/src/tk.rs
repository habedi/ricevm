//! Built-in Tk module ($Tk) implementation.
//!
//! Provides the Inferno Tk widget toolkit interface.
//! Core functions: toplevel, cmd, namechan, pointer, keyboard.

use ricevm_core::ExecError;

use crate::builtin::{BuiltinFunc, BuiltinModule};
use crate::heap::{HeapData, HeapId};
use crate::memory;
use crate::vm::VmState;

/// Create the $Tk built-in module.
pub(crate) fn create_tk_module() -> BuiltinModule {
    BuiltinModule {
        name: "$Tk",
        funcs: vec![
            bf("cmd", 0x01ee9697, 48, tk_cmd),
            bf("color", 0, 40, tk_color),
            bf("getimage", 0, 48, tk_getimage),
            bf("keyboard", 0x8671bae6, 48, tk_keyboard),
            bf("namechan", 0x35182638, 56, tk_namechan),
            bf("pointer", 0x21188625, 48, tk_pointer),
            bf("putimage", 0x2dc55622, 56, tk_putimage),
            bf("quote", 0, 40, tk_quote),
            bf("rect", 0x683e6bae, 56, tk_rect),
            bf("toplevel", 0x96ab1cc9, 48, tk_toplevel),
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

fn tk_getimage(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let toplevel_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    // Return the toplevel's image (offset 8 in Toplevel record)
    let img = if let Some(obj) = vm.heap.get(toplevel_id) {
        if let HeapData::Record(data) = &obj.data {
            if data.len() >= 12 {
                memory::read_word(data, 8) as HeapId
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0
    };
    memory::write_word(&mut vm.frames.data, frame_base, img as i32);
    Ok(())
}

fn tk_putimage(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // putimage replaces the toplevel's image; success = 0
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

fn tk_rect(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Return bounding rectangle: Rect(0, 0, 800, 600) = 4 words at frame+0
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    memory::write_word(&mut vm.frames.data, frame_base + 4, 0);
    memory::write_word(&mut vm.frames.data, frame_base + 8, 800);
    memory::write_word(&mut vm.frames.data, frame_base + 12, 600);
    Ok(())
}

/// Tk->toplevel: create a top-level window.
/// Returns a ref Toplevel (heap record with display, image, etc.)
fn tk_toplevel(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let _display_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let arg_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;
    let _arg = vm.heap.get_string(arg_id).unwrap_or("").to_string();

    tracing::trace!(arg = _arg, "Tk.toplevel");

    // Create a Toplevel record:
    // Offset 0: display (pointer)
    // Offset 4: wreq (channel pointer)
    // Offset 8: image (pointer)
    // Offset 12: ctxt (pointer)
    // Offset 16..32: screenr (Rect = 4 ints)
    let mut toplevel_data = vec![0u8; 48];
    memory::write_word(&mut toplevel_data, 0, _display_id as i32); // display
    // wreq channel
    let chan_id = vm.heap.alloc(
        0,
        HeapData::Channel {
            elem_size: 4,
            pending: None,
        },
    );
    memory::write_word(&mut toplevel_data, 4, chan_id as i32);
    // image: create a window image
    let img = crate::draw::make_image(&mut vm.heap, 0, 0, 800, 600, 32, _display_id);
    memory::write_word(&mut toplevel_data, 8, img as i32);
    // screenr: 0,0,800,600
    memory::write_word(&mut toplevel_data, 16, 0);
    memory::write_word(&mut toplevel_data, 20, 0);
    memory::write_word(&mut toplevel_data, 24, 800);
    memory::write_word(&mut toplevel_data, 28, 600);

    let tl_id = vm.heap.alloc(0, HeapData::Record(toplevel_data));
    // Write result at frame offset 0 (standard return location)
    memory::write_word(&mut vm.frames.data, frame_base, tl_id as i32);
    // Write at the return pointer location (offset 16 holds the caller's return address)
    let ret_addr = memory::read_word(&vm.frames.data, frame_base + 16);
    tracing::trace!(
        tl_id = tl_id,
        frame_base = frame_base,
        ret_addr = ret_addr,
        stack_len = vm.frames.data.len(),
        "Tk.toplevel: writing return value"
    );
    if ret_addr > 0 && (ret_addr as usize) + 4 <= vm.frames.data.len() {
        memory::write_word(&mut vm.frames.data, ret_addr as usize, tl_id as i32);
    }
    Ok(())
}

/// Tk->cmd: execute a Tk command string.
/// Returns a result string (empty on success, error message on failure).
fn tk_cmd(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let _toplevel_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let cmd_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;
    let cmd = vm.heap.get_string(cmd_id).unwrap_or("").to_string();

    tracing::debug!(cmd = cmd, "Tk.cmd");

    #[cfg(feature = "gui")]
    {
        // Parse basic Tk commands and render via SDL2
        process_tk_cmd(vm, &cmd);
    }

    // Return empty string (success)
    let result_id = vm.heap.alloc(0, HeapData::Str(String::new()));
    memory::write_word(&mut vm.frames.data, frame_base, result_id as i32);
    Ok(())
}

/// Tk->namechan: register a named channel for Tk events.
fn tk_namechan(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let _toplevel_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let _chan_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;
    let name_id = memory::read_word(&vm.frames.data, frame_base + 24) as HeapId;
    let _name = vm.heap.get_string(name_id).unwrap_or("").to_string();

    tracing::debug!(name = _name, "Tk.namechan");

    // Return empty string (success)
    let result_id = vm.heap.alloc(0, HeapData::Str(String::new()));
    memory::write_word(&mut vm.frames.data, frame_base, result_id as i32);
    Ok(())
}

/// Tk->pointer: create a Pointer ADT record.
fn tk_pointer(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Return a Pointer record: (x: int, y: int, buttons: int, msec: int) = 16 bytes
    let data = vec![0u8; 16];
    let ptr_id = vm.heap.alloc(0, HeapData::Record(data));
    memory::write_word(&mut vm.frames.data, frame_base, ptr_id as i32);
    Ok(())
}

/// Tk->keyboard: send a keyboard event to Tk.
fn tk_keyboard(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Return 0 (no key pressed)
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

/// Tk->quote: quote a string for Tk.
fn tk_quote(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let str_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let s = vm.heap.get_string(str_id).unwrap_or("").to_string();
    let quoted = format!("{{{s}}}");
    let result_id = vm.heap.alloc(0, HeapData::Str(quoted));
    memory::write_word(&mut vm.frames.data, frame_base, result_id as i32);
    Ok(())
}

/// Tk->color: convert a color name to an integer.
fn tk_color(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let str_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let s = vm.heap.get_string(str_id).unwrap_or("").to_string();

    let color = match s.as_str() {
        "black" => 0x000000FF_u32,
        "white" => 0xFFFFFFFF,
        "red" => 0xFF0000FF,
        "green" => 0x00FF00FF,
        "blue" => 0x0000FFFF,
        "yellow" => 0xFFFF00FF,
        "cyan" => 0x00FFFFFF,
        "magenta" => 0xFF00FFFF,
        _ => {
            // Try parsing as hex: #RRGGBB
            if s.starts_with('#') && s.len() == 7 {
                u32::from_str_radix(&s[1..], 16).unwrap_or(0) << 8 | 0xFF
            } else {
                0xFFFFFFFF
            }
        }
    };
    memory::write_word(&mut vm.frames.data, frame_base, color as i32);
    Ok(())
}

/// Widget tree for Tk rendering.
#[cfg(feature = "gui")]
pub(crate) mod widgets {
    use std::collections::HashMap;

    #[derive(Debug, Clone, Copy)]
    pub enum PackSide {
        Top,
        Bottom,
        Left,
        Right,
    }

    #[derive(Debug, Clone)]
    pub enum WidgetKind {
        Frame,
        Label { text: String },
        Button { text: String, command: String },
        Entry { text: String },
        Canvas { width: i32, height: i32 },
        Text { content: String },
    }

    #[derive(Debug, Clone)]
    pub struct Widget {
        pub name: String,
        pub kind: WidgetKind,
        pub x: i32,
        pub y: i32,
        pub w: i32,
        pub h: i32,
        pub bg_color: u32,
        pub fg_color: u32,
        pub children: Vec<String>,
        pub pack_side: PackSide,
    }

    #[derive(Debug, Default)]
    pub struct WidgetTree {
        pub widgets: HashMap<String, Widget>,
        pub root_children: Vec<String>,
        pub needs_layout: bool,
    }

    impl WidgetTree {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn add_widget(&mut self, name: String, kind: WidgetKind) {
            let widget = Widget {
                name: name.clone(),
                kind,
                x: 0,
                y: 0,
                w: 100,
                h: 24,
                bg_color: 0xDDDDDDFF,
                fg_color: 0x000000FF,
                children: Vec::new(),
                pack_side: PackSide::Top,
            };
            // Find parent widget
            let parent = parent_name(&name);
            if let Some(p) = self.widgets.get_mut(&parent) {
                p.children.push(name.clone());
            } else if parent == "." {
                self.root_children.push(name.clone());
            }
            self.widgets.insert(name, widget);
            self.needs_layout = true;
        }

        pub fn pack(&mut self, name: &str, side: PackSide) {
            if let Some(w) = self.widgets.get_mut(name) {
                w.pack_side = side;
            }
            self.needs_layout = true;
        }

        pub fn layout(&mut self, width: i32, height: i32) {
            layout_children(&self.root_children.clone(), &mut self.widgets, 0, 0, width, height);
            self.needs_layout = false;
        }
    }

    fn parent_name(name: &str) -> String {
        if let Some(pos) = name.rfind('.') {
            if pos == 0 {
                ".".to_string()
            } else {
                name[..pos].to_string()
            }
        } else {
            ".".to_string()
        }
    }

    fn layout_children(
        children: &[String],
        widgets: &mut HashMap<String, Widget>,
        mut x: i32,
        mut y: i32,
        avail_w: i32,
        avail_h: i32,
    ) {
        let mut remaining_w = avail_w;
        let mut remaining_h = avail_h;

        for name in children {
            let (side, child_names) = if let Some(w) = widgets.get(name) {
                (w.pack_side, w.children.clone())
            } else {
                continue;
            };

            let (ww, hh) = match side {
                PackSide::Top | PackSide::Bottom => (remaining_w, 24.min(remaining_h)),
                PackSide::Left | PackSide::Right => (100.min(remaining_w), remaining_h),
            };

            if let Some(w) = widgets.get_mut(name) {
                w.x = x;
                w.y = y;
                w.w = ww;
                w.h = hh;
            }

            if !child_names.is_empty() {
                layout_children(&child_names, widgets, x, y, ww, hh);
            }

            match side {
                PackSide::Top => {
                    y += hh;
                    remaining_h -= hh;
                }
                PackSide::Bottom => {
                    remaining_h -= hh;
                }
                PackSide::Left => {
                    x += ww;
                    remaining_w -= ww;
                }
                PackSide::Right => {
                    remaining_w -= ww;
                }
            }
        }
    }
}

#[cfg(feature = "gui")]
thread_local! {
    static WIDGET_TREE: std::cell::RefCell<widgets::WidgetTree> =
        std::cell::RefCell::new(widgets::WidgetTree::new());
}

/// Process a Tk command string via SDL2.
#[cfg(feature = "gui")]
fn process_tk_cmd(vm: &mut VmState<'_>, cmd: &str) {
    use sdl2::pixels::Color;
    use sdl2::rect::Rect as SdlRect;

    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }

    let first = parts[0];

    match first {
        "update" => {
            // Layout and render all widgets
            WIDGET_TREE.with(|tree| {
                let mut tree = tree.borrow_mut();
                if tree.needs_layout {
                    tree.layout(800, 600);
                }
                super::draw::state::with(|opt_state| {
                    if let Some(state) = opt_state {
                        state.canvas.set_draw_color(Color::RGB(0xDD, 0xDD, 0xDD));
                        state.canvas.clear();
                        for w in tree.widgets.values() {
                            let bg = inferno_to_sdl_color(w.bg_color);
                            state.canvas.set_draw_color(bg);
                            let _ = state.canvas.fill_rect(SdlRect::new(
                                w.x,
                                w.y,
                                w.w as u32,
                                w.h as u32,
                            ));
                            // Draw text for labels and buttons
                            match &w.kind {
                                widgets::WidgetKind::Label { text }
                                | widgets::WidgetKind::Button { text, .. } => {
                                    let fg = inferno_to_sdl_color(w.fg_color);
                                    state.canvas.set_draw_color(fg);
                                    // Monospace text approximation: 8px per char
                                    for (i, _ch) in text.chars().enumerate() {
                                        let cx = w.x + 4 + (i as i32) * 8;
                                        let cy = w.y + 4;
                                        let _ = state
                                            .canvas
                                            .fill_rect(SdlRect::new(cx, cy, 6, 12));
                                    }
                                }
                                widgets::WidgetKind::Button { .. } => {
                                    state.canvas.set_draw_color(Color::BLACK);
                                    let _ = state.canvas.draw_rect(SdlRect::new(
                                        w.x,
                                        w.y,
                                        w.w as u32,
                                        w.h as u32,
                                    ));
                                }
                                _ => {}
                            }
                        }
                        state.canvas.present();
                    }
                });
            });
        }
        "pack" => {
            // pack .name -side top|bottom|left|right
            if let Some(name) = parts.get(1) {
                let side = parse_pack_side(&parts);
                WIDGET_TREE.with(|tree| {
                    tree.borrow_mut().pack(name, side);
                });
            }
        }
        "label" | "button" | "entry" | "text" | "canvas" | "frame" => {
            if let Some(name) = parts.get(1) {
                let text = extract_option(&parts, "-text").unwrap_or_default();
                let command = extract_option(&parts, "-command").unwrap_or_default();
                let kind = match first {
                    "label" => widgets::WidgetKind::Label { text },
                    "button" => widgets::WidgetKind::Button { text, command },
                    "entry" => widgets::WidgetKind::Entry { text },
                    "canvas" => widgets::WidgetKind::Canvas {
                        width: extract_int_option(&parts, "-width").unwrap_or(200),
                        height: extract_int_option(&parts, "-height").unwrap_or(200),
                    },
                    "text" => widgets::WidgetKind::Text {
                        content: String::new(),
                    },
                    _ => widgets::WidgetKind::Frame,
                };
                let bg = extract_color_option(&parts, "-bg");
                WIDGET_TREE.with(|tree| {
                    let mut tree = tree.borrow_mut();
                    tree.add_widget(name.to_string(), kind);
                    if let Some(color) = bg {
                        if let Some(w) = tree.widgets.get_mut(*name) {
                            w.bg_color = color;
                        }
                    }
                });
            }
        }
        _ => {
            tracing::trace!(cmd = cmd, "Tk.cmd (unhandled)");
        }
    }
}

#[cfg(feature = "gui")]
fn inferno_to_sdl_color(c: u32) -> sdl2::pixels::Color {
    sdl2::pixels::Color::RGBA(
        ((c >> 24) & 0xFF) as u8,
        ((c >> 16) & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
        (c & 0xFF) as u8,
    )
}

#[cfg(feature = "gui")]
fn parse_pack_side(parts: &[&str]) -> widgets::PackSide {
    for i in 0..parts.len().saturating_sub(1) {
        if parts[i] == "-side" {
            return match parts[i + 1] {
                "bottom" => widgets::PackSide::Bottom,
                "left" => widgets::PackSide::Left,
                "right" => widgets::PackSide::Right,
                _ => widgets::PackSide::Top,
            };
        }
    }
    widgets::PackSide::Top
}

#[cfg(feature = "gui")]
fn extract_option(parts: &[&str], opt: &str) -> Option<String> {
    for i in 0..parts.len().saturating_sub(1) {
        if parts[i] == opt {
            return Some(parts[i + 1].trim_matches('"').trim_matches('{').trim_matches('}').to_string());
        }
    }
    None
}

#[cfg(feature = "gui")]
fn extract_int_option(parts: &[&str], opt: &str) -> Option<i32> {
    extract_option(parts, opt)?.parse().ok()
}

#[cfg(feature = "gui")]
fn extract_color_option(parts: &[&str], opt: &str) -> Option<u32> {
    let val = extract_option(parts, opt)?;
    match val.as_str() {
        "white" => Some(0xFFFFFFFF),
        "black" => Some(0x000000FF),
        "red" => Some(0xFF0000FF),
        "green" => Some(0x00FF00FF),
        "blue" => Some(0x0000FFFF),
        "gray" | "grey" => Some(0xBBBBBBFF),
        _ => None,
    }
}
