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
    let toplevel_id = memory::read_word(&vm.frames.data, frame_base + 32) as HeapId;
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
    let _display_id = memory::read_word(&vm.frames.data, frame_base + 32) as HeapId;
    let arg_id = memory::read_word(&vm.frames.data, frame_base + 36) as HeapId;
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
    let ret_addr = memory::read_word(&vm.frames.data, frame_base + 32);
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
    let toplevel_id = memory::read_word(&vm.frames.data, frame_base + 32) as HeapId;
    let cmd_id = memory::read_word(&vm.frames.data, frame_base + 36) as HeapId;
    let cmd = vm.heap.get_string(cmd_id).unwrap_or("").to_string();

    tracing::debug!(cmd = cmd, "Tk.cmd");

    let result = dispatch_tk_cmd(vm, toplevel_id, &cmd);

    let result_id = vm.heap.alloc(0, HeapData::Str(result));
    memory::write_word(&mut vm.frames.data, frame_base, result_id as i32);
    Ok(())
}

// Named channels registered via Tk->namechan (maps channel name to heap ID).
thread_local! {
    static NAMED_CHANNELS: std::cell::RefCell<std::collections::HashMap<String, HeapId>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

// Widget property storage for cget/configure.
thread_local! {
    static WIDGET_PROPS: std::cell::RefCell<std::collections::HashMap<String, std::collections::HashMap<String, String>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Dispatch a Tk command and return a result string.
fn dispatch_tk_cmd(vm: &mut VmState<'_>, _toplevel_id: HeapId, cmd: &str) -> String {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return String::new();
    }

    // Handle compound commands separated by semicolons
    if cmd.contains(';') {
        let mut last_result = String::new();
        for sub in cmd.split(';') {
            let sub = sub.trim();
            if !sub.is_empty() {
                last_result = dispatch_tk_cmd(vm, _toplevel_id, sub);
            }
        }
        return last_result;
    }

    let parts: Vec<&str> = shell_split(cmd);
    if parts.is_empty() {
        return String::new();
    }

    let first = parts[0];

    // Widget subcommand: .name configure/cget/...
    if first.starts_with('.') {
        return dispatch_widget_cmd(&parts);
    }

    match first {
        "update" | "frame" | "label" | "button" | "entry" | "text" | "canvas" => {
            #[cfg(feature = "gui")]
            process_tk_cmd(vm, cmd);
            String::new()
        }
        "pack" => {
            if parts.get(1).is_some_and(|s| *s == "propagate") {
                // pack propagate . 0 — ignore
                return String::new();
            }
            #[cfg(feature = "gui")]
            process_tk_cmd(vm, cmd);
            String::new()
        }
        "bind" => {
            // bind .widget <Event> {command} — store but don't act
            String::new()
        }
        "send" => {
            // send channame value — send a string to a named channel
            if let Some(chan_name) = parts.get(1) {
                let value = if parts.len() > 2 {
                    parts[2..].join(" ")
                } else {
                    String::new()
                };
                send_to_named_channel(vm, chan_name, &value);
            }
            String::new()
        }
        "winfo" => {
            // winfo class .name — return widget class name
            if parts.get(1).is_some_and(|s| *s == "class")
                && let Some(name) = parts.get(2)
            {
                return widget_class(name);
            }
            // winfo exists .name
            if parts.get(1).is_some_and(|s| *s == "exists") {
                return "1".to_string();
            }
            String::new()
        }
        "destroy" | "focus" | "raise" | "lower" | "grab" | "selection" | "image" => {
            // Window management commands — accept silently
            String::new()
        }
        _ => {
            tracing::trace!(cmd = cmd, "Tk.cmd (unhandled)");
            String::new()
        }
    }
}

/// Handle .widget subcommands (configure, cget, etc.)
fn dispatch_widget_cmd(parts: &[&str]) -> String {
    let name = parts[0];
    let subcmd = parts.get(1).copied().unwrap_or("");

    match subcmd {
        "configure" => {
            // .name configure -opt val ...
            WIDGET_PROPS.with(|props| {
                let mut props = props.borrow_mut();
                let entry = props.entry(name.to_string()).or_default();
                let mut i = 2;
                while i + 1 < parts.len() {
                    let opt = parts[i].trim_start_matches('-');
                    let val = parts[i + 1]
                        .trim_matches('\'')
                        .trim_matches('{')
                        .trim_matches('}');
                    entry.insert(opt.to_string(), val.to_string());
                    i += 2;
                }
            });
            #[cfg(feature = "gui")]
            {
                // Update widget in the widget tree if text changed
                let text = find_option(parts, "-text");
                let bg = find_option(parts, "-bg");
                if text.is_some() || bg.is_some() {
                    WIDGET_TREE.with(|tree| {
                        let mut tree = tree.borrow_mut();
                        if let Some(w) = tree.widgets.get_mut(name) {
                            if let Some(t) = text {
                                match &mut w.kind {
                                    widgets::WidgetKind::Label { text } => *text = t,
                                    widgets::WidgetKind::Button { text, .. } => *text = t,
                                    _ => {}
                                }
                            }
                            if let Some(b) = bg
                                && let Some(c) = parse_hex_color(&b)
                            {
                                w.bg_color = c;
                            }
                            tree.needs_layout = true;
                        }
                    });
                }
            }
            String::new()
        }
        "cget" => {
            // .name cget -opt
            if let Some(opt) = parts.get(2) {
                let opt = opt.trim_start_matches('-');
                WIDGET_PROPS.with(|props| {
                    props
                        .borrow()
                        .get(name)
                        .and_then(|p| p.get(opt))
                        .cloned()
                        .unwrap_or_default()
                })
            } else {
                String::new()
            }
        }
        _ => {
            // Unknown widget command; not an error, just return empty
            tracing::trace!(name, subcmd, "Tk widget cmd (unhandled)");
            String::new()
        }
    }
}

/// Return the Tk widget class name (capitalized widget type).
fn widget_class(_name: &str) -> String {
    #[cfg(feature = "gui")]
    {
        WIDGET_TREE.with(|tree| {
            let tree = tree.borrow();
            if let Some(w) = tree.widgets.get(_name) {
                match &w.kind {
                    widgets::WidgetKind::Frame => "Frame".to_string(),
                    widgets::WidgetKind::Label { .. } => "Label".to_string(),
                    widgets::WidgetKind::Button { .. } => "Button".to_string(),
                    widgets::WidgetKind::Entry { .. } => "Entry".to_string(),
                    widgets::WidgetKind::Canvas { .. } => "Canvas".to_string(),
                    widgets::WidgetKind::Text { .. } => "Text".to_string(),
                }
            } else {
                format!("!widget {_name} not found")
            }
        })
    }
    #[cfg(not(feature = "gui"))]
    {
        format!("!widget {_name} not found")
    }
}

/// Send a string value on a named Tk channel.
fn send_to_named_channel(vm: &mut VmState<'_>, name: &str, value: &str) {
    let chan_id = NAMED_CHANNELS.with(|nc| nc.borrow().get(name).copied());
    if let Some(id) = chan_id {
        let str_id = vm.heap.alloc(0, HeapData::Str(value.to_string()));
        // Write the string pointer as the channel's pending payload
        let mut payload = vec![0u8; 4];
        memory::write_word(&mut payload, 0, str_id as i32);
        if let Some(obj) = vm.heap.get_mut(id)
            && let HeapData::Channel { pending, .. } = &mut obj.data
        {
            *pending = Some(payload);
        }
        // Unblock any thread waiting on this channel
        vm.unblock_channel(id);
    }
}

/// Simple shell-like splitting that handles {braces} and 'quotes'.
fn shell_split(s: &str) -> Vec<&str> {
    // For simplicity, just split on whitespace but track brace depth
    let mut result = Vec::new();
    let mut start = None;
    let mut brace_depth = 0i32;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => {
                if brace_depth == 0 && start.is_none() {
                    start = Some(i + 1);
                }
                brace_depth += 1;
            }
            b'}' => {
                brace_depth -= 1;
                if brace_depth == 0
                    && let Some(st) = start
                {
                    result.push(&s[st..i]);
                    start = None;
                }
            }
            b' ' | b'\t' if brace_depth == 0 => {
                if let Some(st) = start {
                    result.push(&s[st..i]);
                    start = None;
                }
            }
            _ => {
                if start.is_none() && brace_depth == 0 {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(st) = start {
        result.push(&s[st..]);
    }
    result
}

/// Find an option value in parts list, handling quotes and braces.
fn find_option(parts: &[&str], opt: &str) -> Option<String> {
    for i in 0..parts.len().saturating_sub(1) {
        if parts[i] == opt {
            return Some(
                parts[i + 1]
                    .trim_matches('\'')
                    .trim_matches('{')
                    .trim_matches('}')
                    .to_string(),
            );
        }
    }
    None
}

/// Parse a hex color string like "#aaaaaa" or "#ff5500" to RGBA u32.
fn parse_hex_color(s: &str) -> Option<u32> {
    let s = s.strip_prefix('#')?;
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF)
    } else {
        None
    }
}

/// Tk->namechan: register a named channel for Tk events.
fn tk_namechan(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let _toplevel_id = memory::read_word(&vm.frames.data, frame_base + 32) as HeapId;
    let chan_id = memory::read_word(&vm.frames.data, frame_base + 36) as HeapId;
    let name_id = memory::read_word(&vm.frames.data, frame_base + 40) as HeapId;
    let name = vm.heap.get_string(name_id).unwrap_or("").to_string();

    tracing::debug!(name = name, chan_id = chan_id, "Tk.namechan");

    // Store the channel mapping
    NAMED_CHANNELS.with(|nc| {
        nc.borrow_mut().insert(name, chan_id);
    });

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
    let str_id = memory::read_word(&vm.frames.data, frame_base + 32) as HeapId;
    let s = vm.heap.get_string(str_id).unwrap_or("").to_string();
    let quoted = format!("{{{s}}}");
    let result_id = vm.heap.alloc(0, HeapData::Str(quoted));
    memory::write_word(&mut vm.frames.data, frame_base, result_id as i32);
    Ok(())
}

/// Tk->color: convert a color name to an integer.
fn tk_color(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let str_id = memory::read_word(&vm.frames.data, frame_base + 32) as HeapId;
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

        pub fn reparent(&mut self, child: &str, new_parent: &str) {
            // Remove from old parent's children list
            let old_parent = parent_name(child);
            if let Some(p) = self.widgets.get_mut(&old_parent) {
                p.children.retain(|c| c != child);
            } else {
                self.root_children.retain(|c| c != child);
            }
            // Add to new parent's children list
            if let Some(p) = self.widgets.get_mut(new_parent)
                && !p.children.contains(&child.to_string())
            {
                p.children.push(child.to_string());
            }
            self.needs_layout = true;
        }

        pub fn layout(&mut self, width: i32, height: i32) {
            layout_children(
                &self.root_children.clone(),
                &mut self.widgets,
                0,
                0,
                width,
                height,
            );
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

    const CHAR_W: i32 = 8;
    const CHAR_H: i32 = 16;
    const PAD: i32 = 4;

    /// Compute the natural (minimum) size of a widget based on its content.
    fn natural_size(name: &str, widgets: &HashMap<String, Widget>) -> (i32, i32) {
        let w = match widgets.get(name) {
            Some(w) => w,
            None => return (0, 0),
        };
        if !w.children.is_empty() {
            // Frame: size is sum of children
            let mut total_w = 0i32;
            let mut total_h = 0i32;
            for child in &w.children {
                let (cw, ch) = natural_size(child, widgets);
                let side = widgets
                    .get(child)
                    .map(|c| c.pack_side)
                    .unwrap_or(PackSide::Top);
                match side {
                    PackSide::Top | PackSide::Bottom => {
                        total_w = total_w.max(cw);
                        total_h += ch;
                    }
                    PackSide::Left | PackSide::Right => {
                        total_w += cw;
                        total_h = total_h.max(ch);
                    }
                }
            }
            (total_w, total_h)
        } else {
            // Leaf widget: size from text content
            let text_len = match &w.kind {
                WidgetKind::Label { text } | WidgetKind::Button { text, .. } => text.len() as i32,
                WidgetKind::Canvas { width, height } => return (*width, *height),
                _ => 0,
            };
            let tw = text_len * CHAR_W + PAD * 2;
            let th = if text_len > 0 {
                CHAR_H + PAD * 2
            } else {
                CHAR_H
            };
            (tw.max(CHAR_W), th)
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

            let (nat_w, nat_h) = natural_size(name, widgets);

            let (ww, hh) = match side {
                PackSide::Top | PackSide::Bottom => {
                    (remaining_w, nat_h.max(CHAR_H).min(remaining_h))
                }
                PackSide::Left | PackSide::Right => {
                    (nat_w.max(CHAR_W).min(remaining_w), remaining_h)
                }
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
fn process_tk_cmd(_vm: &mut VmState<'_>, cmd: &str) {
    use sdl2::pixels::Color;
    use sdl2::rect::Rect as SdlRect;

    let parts: Vec<&str> = shell_split(cmd);
    if parts.is_empty() {
        return;
    }

    let first = parts[0];

    match first {
        "update" => {
            // Ensure SDL2 window exists
            super::draw::state::ensure_init("RiceVM", 400, 300);
            // Layout and render all widgets; collect pending button commands
            let mut pending_cmds: Vec<String> = Vec::new();
            WIDGET_TREE.with(|tree| {
                let mut tree = tree.borrow_mut();
                if tree.needs_layout {
                    tree.layout(400, 300);
                }
                super::draw::state::with(|opt_state| {
                    if let Some(state) = opt_state {
                        state.canvas.set_draw_color(Color::RGB(200, 200, 200));
                        state.canvas.clear();
                        for w in tree.widgets.values() {
                            let bg = inferno_to_sdl_color(w.bg_color);
                            state.canvas.set_draw_color(bg);
                            let _ = state
                                .canvas
                                .fill_rect(SdlRect::new(w.x, w.y, w.w as u32, w.h as u32));
                            // Draw text for labels and buttons
                            match &w.kind {
                                widgets::WidgetKind::Label { text }
                                | widgets::WidgetKind::Button { text, .. } => {
                                    let fg = inferno_to_sdl_color(w.fg_color);
                                    draw_text(&mut state.canvas, text, w.x + 4, w.y + 2, fg);
                                }
                                _ => {}
                            }
                            // Draw button border
                            if matches!(&w.kind, widgets::WidgetKind::Button { .. }) {
                                state.canvas.set_draw_color(Color::BLACK);
                                let _ = state
                                    .canvas
                                    .draw_rect(SdlRect::new(w.x, w.y, w.w as u32, w.h as u32));
                            }
                        }
                        state.canvas.present();
                        // Poll events and dispatch to widgets
                        for event in state.event_pump.poll_iter() {
                            match event {
                                sdl2::event::Event::Quit { .. } => {
                                    std::process::exit(0);
                                }
                                sdl2::event::Event::MouseButtonDown { x, y, .. } => {
                                    // Find which button was clicked
                                    let clicked_cmd = tree
                                        .widgets
                                        .values()
                                        .find(|w| {
                                            matches!(&w.kind, widgets::WidgetKind::Button { .. })
                                                && x >= w.x
                                                && x < w.x + w.w
                                                && y >= w.y
                                                && y < w.y + w.h
                                        })
                                        .and_then(|w| {
                                            if let widgets::WidgetKind::Button { command, .. } =
                                                &w.kind
                                            {
                                                if !command.is_empty() {
                                                    Some(command.clone())
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        });
                                    if let Some(cmd) = clicked_cmd {
                                        pending_cmds.push(cmd);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                });
            });
            // Dispatch pending button commands (outside tree borrow)
            for cmd in pending_cmds {
                // Button commands are Tk send commands like "send wm_title exit"
                let parts: Vec<&str> = shell_split(&cmd);
                if parts.first().is_some_and(|s| *s == "send") {
                    if let Some(chan_name) = parts.get(1) {
                        let value = if parts.len() > 2 {
                            parts[2..].join(" ")
                        } else {
                            String::new()
                        };
                        send_to_named_channel(_vm, chan_name, &value);
                    }
                }
            }
        }
        "pack" => {
            // pack .name1 .name2 ... -side top -in .parent
            let side = parse_pack_side(&parts);
            let parent = extract_option_str(&parts, "-in");
            // Collect widget names (arguments before options)
            for part in parts.iter().skip(1) {
                if part.starts_with('-') {
                    break;
                }
                let name = *part;
                WIDGET_TREE.with(|tree| {
                    let mut tree = tree.borrow_mut();
                    tree.pack(name, side);
                    // Handle -in: re-parent the widget
                    if let Some(ref parent_name) = parent {
                        tree.reparent(name, parent_name);
                    }
                });
            }
        }
        "label" | "button" | "entry" | "text" | "canvas" | "frame" => {
            if let Some(name) = parts.get(1) {
                let text = extract_option_str(&parts, "-text").unwrap_or_default();
                let command = extract_option_str(&parts, "-command").unwrap_or_default();
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
                let fg = extract_color_option(&parts, "-fg");
                WIDGET_TREE.with(|tree| {
                    let mut tree = tree.borrow_mut();
                    tree.add_widget(name.to_string(), kind);
                    if let Some(w) = tree.widgets.get_mut(*name) {
                        if let Some(color) = bg {
                            w.bg_color = color;
                        }
                        if let Some(color) = fg {
                            w.fg_color = color;
                        }
                    }
                });
                // Also store properties for cget
                WIDGET_PROPS.with(|props| {
                    let mut props = props.borrow_mut();
                    let entry = props.entry(name.to_string()).or_default();
                    if let Some(t) = extract_option_str(&parts, "-text") {
                        entry.insert("text".to_string(), t);
                    }
                    if let Some(b) = extract_option_str(&parts, "-bg") {
                        entry.insert("bg".to_string(), b);
                    }
                    if let Some(f) = extract_option_str(&parts, "-fg") {
                        entry.insert("fg".to_string(), f);
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
    // Inferno color format: 0xRRGGBBAA
    sdl2::pixels::Color::RGB(
        ((c >> 24) & 0xFF) as u8,
        ((c >> 16) & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
    )
}

/// Draw a text string on the SDL2 canvas using a simple bitmap font.
/// Each character is rendered as a 7x13 glyph using point-by-point drawing.
#[cfg(feature = "gui")]
fn draw_text(
    canvas: &mut sdl2::render::Canvas<sdl2::video::Window>,
    text: &str,
    x: i32,
    y: i32,
    color: sdl2::pixels::Color,
) {
    canvas.set_draw_color(color);
    let glyph_w = 8i32;
    let glyph_h = 13i32;
    for (i, ch) in text.chars().enumerate() {
        let cx = x + (i as i32) * glyph_w;
        if let Some(glyph) = bitmap_font::glyph(ch) {
            for (row, &bits) in glyph.iter().enumerate() {
                for col in 0..8u32 {
                    if bits & (0x80 >> col) != 0 {
                        let _ = canvas
                            .draw_point(sdl2::rect::Point::new(cx + col as i32, y + row as i32));
                    }
                }
            }
        } else {
            // Unknown character: draw a small box
            let _ = canvas.draw_rect(sdl2::rect::Rect::new(cx, y + 1, 6, glyph_h as u32 - 2));
        }
    }
}

/// Embedded 8x13 bitmap font for printable ASCII (based on fixed/misc-fixed style).
/// Each glyph is 13 bytes; each byte is one row where bit 7 = leftmost pixel.
#[cfg(feature = "gui")]
mod bitmap_font {
    pub fn glyph(ch: char) -> Option<&'static [u8; 13]> {
        let idx = ch as u32;
        if (32..=126).contains(&idx) {
            Some(&FONT[(idx - 32) as usize])
        } else {
            None
        }
    }

    // Minimal 8x13 bitmap font for ASCII 32-126 (95 glyphs).
    // Each glyph: 13 rows of 8 bits. Bit 7 = leftmost pixel.
    static FONT: [[u8; 13]; 95] = [
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // ' '
        [
            0x00, 0x00, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00,
        ], // '!'
        [
            0x00, 0x00, 0x24, 0x24, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // '"'
        [
            0x00, 0x00, 0x24, 0x24, 0x7E, 0x24, 0x7E, 0x24, 0x24, 0x00, 0x00, 0x00, 0x00,
        ], // '#'
        [
            0x00, 0x00, 0x10, 0x3C, 0x50, 0x38, 0x14, 0x78, 0x10, 0x00, 0x00, 0x00, 0x00,
        ], // '$'
        [
            0x00, 0x00, 0x22, 0x52, 0x24, 0x08, 0x10, 0x24, 0x4A, 0x44, 0x00, 0x00, 0x00,
        ], // '%'
        [
            0x00, 0x00, 0x30, 0x48, 0x48, 0x30, 0x4A, 0x44, 0x3A, 0x00, 0x00, 0x00, 0x00,
        ], // '&'
        [
            0x00, 0x00, 0x10, 0x10, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // '\''
        [
            0x00, 0x00, 0x08, 0x10, 0x20, 0x20, 0x20, 0x20, 0x10, 0x08, 0x00, 0x00, 0x00,
        ], // '('
        [
            0x00, 0x00, 0x20, 0x10, 0x08, 0x08, 0x08, 0x08, 0x10, 0x20, 0x00, 0x00, 0x00,
        ], // ')'
        [
            0x00, 0x00, 0x00, 0x10, 0x54, 0x38, 0x54, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // '*'
        [
            0x00, 0x00, 0x00, 0x10, 0x10, 0x7C, 0x10, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // '+'
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x10, 0x20, 0x00, 0x00,
        ], // ','
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x7C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // '-'
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00,
        ], // '.'
        [
            0x00, 0x00, 0x04, 0x04, 0x08, 0x08, 0x10, 0x20, 0x40, 0x40, 0x00, 0x00, 0x00,
        ], // '/'
        [
            0x00, 0x00, 0x38, 0x44, 0x44, 0x44, 0x44, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // '0'
        [
            0x00, 0x00, 0x10, 0x30, 0x10, 0x10, 0x10, 0x10, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // '1'
        [
            0x00, 0x00, 0x38, 0x44, 0x04, 0x08, 0x10, 0x20, 0x7C, 0x00, 0x00, 0x00, 0x00,
        ], // '2'
        [
            0x00, 0x00, 0x38, 0x44, 0x04, 0x18, 0x04, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // '3'
        [
            0x00, 0x00, 0x08, 0x18, 0x28, 0x48, 0x7C, 0x08, 0x08, 0x00, 0x00, 0x00, 0x00,
        ], // '4'
        [
            0x00, 0x00, 0x7C, 0x40, 0x78, 0x04, 0x04, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // '5'
        [
            0x00, 0x00, 0x18, 0x20, 0x40, 0x78, 0x44, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // '6'
        [
            0x00, 0x00, 0x7C, 0x04, 0x08, 0x10, 0x10, 0x10, 0x10, 0x00, 0x00, 0x00, 0x00,
        ], // '7'
        [
            0x00, 0x00, 0x38, 0x44, 0x44, 0x38, 0x44, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // '8'
        [
            0x00, 0x00, 0x38, 0x44, 0x44, 0x3C, 0x04, 0x08, 0x30, 0x00, 0x00, 0x00, 0x00,
        ], // '9'
        [
            0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // ':'
        [
            0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x10, 0x10, 0x20, 0x00, 0x00, 0x00,
        ], // ';'
        [
            0x00, 0x00, 0x04, 0x08, 0x10, 0x20, 0x10, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00,
        ], // '<'
        [
            0x00, 0x00, 0x00, 0x00, 0x7C, 0x00, 0x7C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // '='
        [
            0x00, 0x00, 0x20, 0x10, 0x08, 0x04, 0x08, 0x10, 0x20, 0x00, 0x00, 0x00, 0x00,
        ], // '>'
        [
            0x00, 0x00, 0x38, 0x44, 0x04, 0x08, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00,
        ], // '?'
        [
            0x00, 0x00, 0x38, 0x44, 0x4C, 0x54, 0x4C, 0x40, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // '@'
        [
            0x00, 0x00, 0x38, 0x44, 0x44, 0x7C, 0x44, 0x44, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'A'
        [
            0x00, 0x00, 0x78, 0x44, 0x44, 0x78, 0x44, 0x44, 0x78, 0x00, 0x00, 0x00, 0x00,
        ], // 'B'
        [
            0x00, 0x00, 0x38, 0x44, 0x40, 0x40, 0x40, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // 'C'
        [
            0x00, 0x00, 0x78, 0x44, 0x44, 0x44, 0x44, 0x44, 0x78, 0x00, 0x00, 0x00, 0x00,
        ], // 'D'
        [
            0x00, 0x00, 0x7C, 0x40, 0x40, 0x78, 0x40, 0x40, 0x7C, 0x00, 0x00, 0x00, 0x00,
        ], // 'E'
        [
            0x00, 0x00, 0x7C, 0x40, 0x40, 0x78, 0x40, 0x40, 0x40, 0x00, 0x00, 0x00, 0x00,
        ], // 'F'
        [
            0x00, 0x00, 0x38, 0x44, 0x40, 0x4C, 0x44, 0x44, 0x3C, 0x00, 0x00, 0x00, 0x00,
        ], // 'G'
        [
            0x00, 0x00, 0x44, 0x44, 0x44, 0x7C, 0x44, 0x44, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'H'
        [
            0x00, 0x00, 0x38, 0x10, 0x10, 0x10, 0x10, 0x10, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // 'I'
        [
            0x00, 0x00, 0x1C, 0x08, 0x08, 0x08, 0x08, 0x48, 0x30, 0x00, 0x00, 0x00, 0x00,
        ], // 'J'
        [
            0x00, 0x00, 0x44, 0x48, 0x50, 0x60, 0x50, 0x48, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'K'
        [
            0x00, 0x00, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x7C, 0x00, 0x00, 0x00, 0x00,
        ], // 'L'
        [
            0x00, 0x00, 0x44, 0x6C, 0x54, 0x54, 0x44, 0x44, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'M'
        [
            0x00, 0x00, 0x44, 0x64, 0x54, 0x4C, 0x44, 0x44, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'N'
        [
            0x00, 0x00, 0x38, 0x44, 0x44, 0x44, 0x44, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // 'O'
        [
            0x00, 0x00, 0x78, 0x44, 0x44, 0x78, 0x40, 0x40, 0x40, 0x00, 0x00, 0x00, 0x00,
        ], // 'P'
        [
            0x00, 0x00, 0x38, 0x44, 0x44, 0x44, 0x54, 0x48, 0x34, 0x00, 0x00, 0x00, 0x00,
        ], // 'Q'
        [
            0x00, 0x00, 0x78, 0x44, 0x44, 0x78, 0x50, 0x48, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'R'
        [
            0x00, 0x00, 0x38, 0x44, 0x40, 0x38, 0x04, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // 'S'
        [
            0x00, 0x00, 0x7C, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x00, 0x00, 0x00, 0x00,
        ], // 'T'
        [
            0x00, 0x00, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // 'U'
        [
            0x00, 0x00, 0x44, 0x44, 0x44, 0x44, 0x28, 0x28, 0x10, 0x00, 0x00, 0x00, 0x00,
        ], // 'V'
        [
            0x00, 0x00, 0x44, 0x44, 0x44, 0x54, 0x54, 0x6C, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'W'
        [
            0x00, 0x00, 0x44, 0x44, 0x28, 0x10, 0x28, 0x44, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'X'
        [
            0x00, 0x00, 0x44, 0x44, 0x28, 0x10, 0x10, 0x10, 0x10, 0x00, 0x00, 0x00, 0x00,
        ], // 'Y'
        [
            0x00, 0x00, 0x7C, 0x04, 0x08, 0x10, 0x20, 0x40, 0x7C, 0x00, 0x00, 0x00, 0x00,
        ], // 'Z'
        [
            0x00, 0x00, 0x38, 0x20, 0x20, 0x20, 0x20, 0x20, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // '['
        [
            0x00, 0x00, 0x40, 0x40, 0x20, 0x10, 0x08, 0x04, 0x04, 0x00, 0x00, 0x00, 0x00,
        ], // '\\'
        [
            0x00, 0x00, 0x38, 0x08, 0x08, 0x08, 0x08, 0x08, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // ']'
        [
            0x00, 0x00, 0x10, 0x28, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // '^'
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7C, 0x00, 0x00, 0x00,
        ], // '_'
        [
            0x00, 0x00, 0x20, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // '`'
        [
            0x00, 0x00, 0x00, 0x00, 0x38, 0x04, 0x3C, 0x44, 0x3C, 0x00, 0x00, 0x00, 0x00,
        ], // 'a'
        [
            0x00, 0x00, 0x40, 0x40, 0x78, 0x44, 0x44, 0x44, 0x78, 0x00, 0x00, 0x00, 0x00,
        ], // 'b'
        [
            0x00, 0x00, 0x00, 0x00, 0x38, 0x44, 0x40, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // 'c'
        [
            0x00, 0x00, 0x04, 0x04, 0x3C, 0x44, 0x44, 0x44, 0x3C, 0x00, 0x00, 0x00, 0x00,
        ], // 'd'
        [
            0x00, 0x00, 0x00, 0x00, 0x38, 0x44, 0x7C, 0x40, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // 'e'
        [
            0x00, 0x00, 0x18, 0x24, 0x20, 0x78, 0x20, 0x20, 0x20, 0x00, 0x00, 0x00, 0x00,
        ], // 'f'
        [
            0x00, 0x00, 0x00, 0x00, 0x3C, 0x44, 0x44, 0x3C, 0x04, 0x38, 0x00, 0x00, 0x00,
        ], // 'g'
        [
            0x00, 0x00, 0x40, 0x40, 0x78, 0x44, 0x44, 0x44, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'h'
        [
            0x00, 0x00, 0x10, 0x00, 0x30, 0x10, 0x10, 0x10, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // 'i'
        [
            0x00, 0x00, 0x08, 0x00, 0x18, 0x08, 0x08, 0x08, 0x48, 0x30, 0x00, 0x00, 0x00,
        ], // 'j'
        [
            0x00, 0x00, 0x40, 0x40, 0x48, 0x50, 0x60, 0x50, 0x48, 0x00, 0x00, 0x00, 0x00,
        ], // 'k'
        [
            0x00, 0x00, 0x30, 0x10, 0x10, 0x10, 0x10, 0x10, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // 'l'
        [
            0x00, 0x00, 0x00, 0x00, 0x68, 0x54, 0x54, 0x54, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'm'
        [
            0x00, 0x00, 0x00, 0x00, 0x78, 0x44, 0x44, 0x44, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'n'
        [
            0x00, 0x00, 0x00, 0x00, 0x38, 0x44, 0x44, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
        ], // 'o'
        [
            0x00, 0x00, 0x00, 0x00, 0x78, 0x44, 0x44, 0x78, 0x40, 0x40, 0x00, 0x00, 0x00,
        ], // 'p'
        [
            0x00, 0x00, 0x00, 0x00, 0x3C, 0x44, 0x44, 0x3C, 0x04, 0x04, 0x00, 0x00, 0x00,
        ], // 'q'
        [
            0x00, 0x00, 0x00, 0x00, 0x58, 0x64, 0x40, 0x40, 0x40, 0x00, 0x00, 0x00, 0x00,
        ], // 'r'
        [
            0x00, 0x00, 0x00, 0x00, 0x3C, 0x40, 0x38, 0x04, 0x78, 0x00, 0x00, 0x00, 0x00,
        ], // 's'
        [
            0x00, 0x00, 0x20, 0x20, 0x78, 0x20, 0x20, 0x24, 0x18, 0x00, 0x00, 0x00, 0x00,
        ], // 't'
        [
            0x00, 0x00, 0x00, 0x00, 0x44, 0x44, 0x44, 0x44, 0x3C, 0x00, 0x00, 0x00, 0x00,
        ], // 'u'
        [
            0x00, 0x00, 0x00, 0x00, 0x44, 0x44, 0x44, 0x28, 0x10, 0x00, 0x00, 0x00, 0x00,
        ], // 'v'
        [
            0x00, 0x00, 0x00, 0x00, 0x44, 0x54, 0x54, 0x54, 0x28, 0x00, 0x00, 0x00, 0x00,
        ], // 'w'
        [
            0x00, 0x00, 0x00, 0x00, 0x44, 0x28, 0x10, 0x28, 0x44, 0x00, 0x00, 0x00, 0x00,
        ], // 'x'
        [
            0x00, 0x00, 0x00, 0x00, 0x44, 0x44, 0x44, 0x3C, 0x04, 0x38, 0x00, 0x00, 0x00,
        ], // 'y'
        [
            0x00, 0x00, 0x00, 0x00, 0x7C, 0x08, 0x10, 0x20, 0x7C, 0x00, 0x00, 0x00, 0x00,
        ], // 'z'
        [
            0x00, 0x00, 0x0C, 0x10, 0x10, 0x20, 0x10, 0x10, 0x0C, 0x00, 0x00, 0x00, 0x00,
        ], // '{'
        [
            0x00, 0x00, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x00, 0x00, 0x00, 0x00,
        ], // '|'
        [
            0x00, 0x00, 0x60, 0x10, 0x10, 0x08, 0x10, 0x10, 0x60, 0x00, 0x00, 0x00, 0x00,
        ], // '}'
        [
            0x00, 0x00, 0x24, 0x54, 0x48, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ], // '~'
    ];
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

/// Extract a string option value from parts, handling braces and quotes.
fn extract_option_str(parts: &[&str], opt: &str) -> Option<String> {
    for i in 0..parts.len().saturating_sub(1) {
        if parts[i] == opt {
            return Some(
                parts[i + 1]
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    None
}

#[cfg(feature = "gui")]
fn extract_int_option(parts: &[&str], opt: &str) -> Option<i32> {
    extract_option_str(parts, opt)?.parse().ok()
}

#[cfg(feature = "gui")]
fn extract_color_option(parts: &[&str], opt: &str) -> Option<u32> {
    let val = extract_option_str(parts, opt)?;
    match val.as_str() {
        "white" => Some(0xFFFFFFFF),
        "black" => Some(0x000000FF),
        "red" => Some(0xFF0000FF),
        "green" => Some(0x00FF00FF),
        "blue" => Some(0x0000FFFF),
        "yellow" => Some(0xFFFF00FF),
        "gray" | "grey" => Some(0xBBBBBBFF),
        _ => parse_hex_color(&val),
    }
}
