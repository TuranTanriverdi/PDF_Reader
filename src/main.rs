#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use egui::{pos2, vec2, Color32, RichText};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

mod outline;

// ───────────────────────────── constants ─────────────────────────────

const APP_NAME: &str = "PDF Reader";
const ACCENT: Color32 = Color32::from_rgb(59, 130, 246);
const ACCENT_DARK: Color32 = Color32::from_rgb(37, 99, 235);

/// Vertical space reserved around every page in continuous mode (label + gap).
const PAGE_TOP: f32 = 28.0;
const PAGE_BOTTOM: f32 = 16.0;
const ROW_PAD: f32 = PAGE_TOP + PAGE_BOTTOM;
const PAGE_MARGIN_X: f32 = 32.0;

const MIN_SCALE: f32 = 0.1;
const MAX_SCALE: f32 = 10.0;
/// Zoom ladder used by the +/- buttons (all values sit on the 0.05 render grid).
const ZOOM_STEPS: &[f32] = &[
    0.1, 0.25, 0.35, 0.5, 0.65, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0,
];

const MAX_TEX_COUNT: usize = 64;
const MAX_TEX_BYTES: usize = 512 * 1024 * 1024;
const MAX_RENDERS_PER_FRAME: usize = 2;
const CONFIG_SAVE_INTERVAL: f64 = 1.0;
const STATUS_LIFETIME: f64 = 6.0;

// ───────────────────────────── types ─────────────────────────────

#[derive(PartialEq, Clone, Copy, Debug)]
enum FitMode {
    None,
    Width,
    Height,
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum PrintRange {
    All,
    Current,
    Custom,
}

#[derive(PartialEq, Clone, Copy, Debug, Serialize, Deserialize, Default)]
enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Serialize, Deserialize, Clone)]
struct FileHistory {
    page: usize,
    scale: f32,
    continuous: bool,
    #[serde(default)]
    rotation: u8,
    #[serde(default)]
    dark_mode: bool,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct AppConfig {
    last_file: Option<String>,
    last_page: usize,
    last_scale: f32,
    continuous: bool,
    recent: Vec<String>,
    #[serde(default)]
    file_history: HashMap<String, FileHistory>,
    /// "Invert page colours" (night reading mode).
    #[serde(default)]
    dark_mode: bool,
    #[serde(default)]
    hand_tool: bool,
    /// Application theme (independent from page inversion).
    #[serde(default)]
    theme: ThemePref,
}

// ───────────────────────────── config I/O ─────────────────────────────

fn config_path() -> PathBuf {
    if let Some(mut p) = dirs::config_dir() {
        p.push("pdf-reader");
        let _ = std::fs::create_dir_all(&p);
        p.push("config.json");
        p
    } else {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return dir.join("pdf-reader-config.json");
            }
        }
        PathBuf::from("pdf-reader-config.json")
    }
}

fn load_config(path: &Path) -> AppConfig {
    if let Ok(data) = std::fs::read_to_string(path) {
        if let Ok(cfg) = serde_json::from_str::<AppConfig>(&data) {
            return cfg;
        }
    }
    AppConfig {
        last_scale: 1.5,
        ..Default::default()
    }
}

/// Atomic save: write to a temp file, then rename over the real one so a crash
/// mid-write can never leave a truncated config behind.
fn save_config(cfg: &AppConfig, path: &Path) {
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

fn cli_pdf_path_from_args() -> Option<PathBuf> {
    for arg in std::env::args().skip(1) {
        if arg.starts_with('-') {
            continue;
        }
        let trimmed = arg.trim_matches('"').trim();
        if trimmed.is_empty() {
            continue;
        }
        let p = PathBuf::from(trimmed);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

// ───────────────────────────── theme ─────────────────────────────

fn apply_style(style: &mut egui::Style, dark: bool) {
    use egui::{CornerRadius, FontId, Stroke, TextStyle};

    style.text_styles.insert(TextStyle::Heading, FontId::proportional(22.0));
    style.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
    style.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));

    style.spacing.item_spacing = vec2(8.0, 6.0);
    style.spacing.button_padding = vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.bar_width = 10.0;

    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let r = CornerRadius::same(6);

    if dark {
        v.panel_fill = Color32::from_rgb(32, 33, 37);
        v.window_fill = Color32::from_rgb(40, 41, 46);
        v.extreme_bg_color = Color32::from_rgb(22, 23, 26);
        v.faint_bg_color = Color32::from_rgb(38, 39, 44);
        v.window_stroke = Stroke::new(1.0, Color32::from_rgb(62, 64, 71));
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(58, 60, 66));
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(50, 52, 58);
        v.widgets.inactive.bg_fill = Color32::from_rgb(50, 52, 58);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(64, 67, 75);
        v.widgets.hovered.bg_fill = Color32::from_rgb(64, 67, 75);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(76, 80, 90);
        v.widgets.active.bg_fill = Color32::from_rgb(76, 80, 90);
        v.selection.bg_fill = ACCENT_DARK;
        v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(190, 215, 255));
        v.hyperlink_color = Color32::from_rgb(120, 170, 255);
    } else {
        v.panel_fill = Color32::from_rgb(245, 246, 249);
        v.window_fill = Color32::WHITE;
        v.extreme_bg_color = Color32::WHITE;
        v.faint_bg_color = Color32::from_rgb(238, 240, 244);
        v.window_stroke = Stroke::new(1.0, Color32::from_rgb(208, 212, 220));
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(220, 223, 230));
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(234, 236, 241);
        v.widgets.inactive.bg_fill = Color32::from_rgb(234, 236, 241);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(220, 224, 232);
        v.widgets.hovered.bg_fill = Color32::from_rgb(220, 224, 232);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(205, 210, 221);
        v.widgets.active.bg_fill = Color32::from_rgb(205, 210, 221);
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.hyperlink_color = ACCENT_DARK;
    }
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    v.widgets.open.corner_radius = r;
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    style.visuals = v;
}

fn apply_theme_pref(ctx: &egui::Context, pref: ThemePref) {
    ctx.set_theme(match pref {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Light => egui::ThemePreference::Light,
        ThemePref::Dark => egui::ThemePreference::Dark,
    });
}

fn canvas_color(dark_ui: bool) -> Color32 {
    if dark_ui {
        Color32::from_rgb(24, 25, 28)
    } else {
        Color32::from_rgb(220, 223, 229)
    }
}

fn page_shadow(rect: egui::Rect, dark_ui: bool) -> Vec<egui::Shape> {
    let alpha = if dark_ui { 34 } else { 14 };
    let mut shapes = Vec::with_capacity(7);
    for i in 0..5 {
        let g = 1.0 + i as f32 * 2.0;
        shapes.push(egui::Shape::rect_filled(
            rect.expand(g).translate(vec2(0.0, 2.0)),
            3.0 + g,
            Color32::from_black_alpha(alpha),
        ));
    }
    let border = if dark_ui {
        Color32::from_rgb(70, 72, 80)
    } else {
        Color32::from_rgb(190, 194, 203)
    };
    shapes.push(egui::Shape::rect_filled(rect.expand(1.0), 1.0, border));
    shapes
}

// ───────────────────────────── small helpers ─────────────────────────────

fn quantize_scale(s: f32) -> f32 {
    (s * 20.0).round() / 20.0
}
fn scale_bits(s: f32) -> u32 {
    (quantize_scale(s) * 100.0) as u32
}

fn step_zoom(scale: f32, up: bool) -> f32 {
    let eps = 0.001;
    if up {
        ZOOM_STEPS.iter().copied().find(|&z| z > scale + eps).unwrap_or(scale)
    } else {
        ZOOM_STEPS.iter().rev().copied().find(|&z| z < scale - eps).unwrap_or(scale)
    }
}

fn file_name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn parent_of(p: &str) -> String {
    Path::new(p)
        .parent()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn parse_page_range(input: &str, total_pages: usize) -> Result<Vec<usize>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter pages like 1,3,5-7".into());
    }
    let mut pages = Vec::new();
    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if part.contains('-') {
            let mut split = part.splitn(2, '-');
            let s_str = split.next().unwrap().trim();
            let e_str = split.next().unwrap_or("").trim();
            if s_str.is_empty() || e_str.is_empty() {
                return Err(format!("Invalid range '{}'", part));
            }
            let s: usize = s_str.parse().map_err(|_| format!("Invalid number '{}'", s_str))?;
            let e: usize = e_str.parse().map_err(|_| format!("Invalid number '{}'", e_str))?;
            if s == 0 || e == 0 {
                return Err("Page numbers start at 1".into());
            }
            let (mut a, mut b) = (s, e);
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            if a > total_pages || b > total_pages {
                return Err(format!("Pages out of range (1-{})", total_pages));
            }
            for n in a..=b {
                pages.push(n - 1);
            }
        } else {
            let n: usize = part.parse().map_err(|_| format!("Invalid number '{}'", part))?;
            if n == 0 {
                return Err("Page numbers start at 1".into());
            }
            if n > total_pages {
                return Err(format!("Page {} out of range (1-{})", n, total_pages));
            }
            pages.push(n - 1);
        }
    }
    if pages.is_empty() {
        return Err("No pages".into());
    }
    pages.sort_unstable();
    pages.dedup();
    Ok(pages)
}

fn format_file_size(size: Option<u64>) -> String {
    match size {
        Some(s) if s < 1024 => format!("{} B", s),
        Some(s) if s < 1024 * 1024 => format!("{:.1} KB", s as f64 / 1024.0),
        Some(s) if s < 1024 * 1024 * 1024 => format!("{:.2} MB", s as f64 / (1024.0 * 1024.0)),
        Some(s) => format!("{:.2} GB", s as f64 / (1024.0 * 1024.0 * 1024.0)),
        None => "Unknown".to_string(),
    }
}

// ───────────────────────────── printing (unchanged) ─────────────────────────────

#[cfg(windows)]
mod print_win {
    use super::PrintRange;
    use super::parse_page_range;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use winapi::um::winspool::{GetDefaultPrinterW, EnumPrintersW, OpenPrinterW, StartDocPrinterW, StartPagePrinter, WritePrinter, EndPagePrinter, EndDocPrinter, ClosePrinter, DOC_INFO_1W, PRINTER_INFO_1W, PRINTER_ENUM_LOCAL, PRINTER_ENUM_CONNECTIONS};
    use winapi::um::winnt::HANDLE;
    use winapi::shared::minwindef::{DWORD, LPBYTE};

    fn to_wide(s: &str) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    pub fn get_default_printer() -> Result<String, String> {
        unsafe {
            let mut needed: DWORD = 0;
            GetDefaultPrinterW(std::ptr::null_mut(), &mut needed);
            if needed == 0 { return Err("No default printer".into()); }
            let mut buf: Vec<u16> = vec![0; needed as usize];
            let ok = GetDefaultPrinterW(buf.as_mut_ptr(), &mut needed);
            if ok == 0 { return Err("GetDefaultPrinterW failed".into()); }
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Ok(OsString::from_wide(&buf[..end]).to_string_lossy().into_owned())
        }
    }

    pub fn enum_printers() -> Vec<String> {
        unsafe {
            let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
            let mut needed: DWORD = 0;
            let mut returned: DWORD = 0;
            EnumPrintersW(flags, std::ptr::null_mut(), 1, std::ptr::null_mut(), 0, &mut needed, &mut returned);
            if needed == 0 { return Vec::new(); }
            let mut buffer = vec![0u8; needed as usize];
            let ok = EnumPrintersW(flags, std::ptr::null_mut(), 1, buffer.as_mut_ptr() as LPBYTE, needed, &mut needed, &mut returned);
            if ok == 0 { return Vec::new(); }
            let mut printers = Vec::new();
            let info_ptr = buffer.as_ptr() as *const PRINTER_INFO_1W;
            for i in 0..returned as usize {
                let info = &*info_ptr.add(i);
                if !info.pName.is_null() {
                    let len = (0..).take_while(|&j| *info.pName.add(j) != 0).count();
                    let slice = std::slice::from_raw_parts(info.pName, len);
                    let name = OsString::from_wide(slice).to_string_lossy().into_owned();
                    if !name.is_empty() { printers.push(name); }
                }
            }
            printers.sort(); printers.dedup(); printers
        }
    }

    fn extract_pages_pdf(original: &[u8], keep_indices: &[usize]) -> Result<Vec<u8>, String> {
        use lopdf::{Document, Object};
        if keep_indices.is_empty() { return Err("No pages".into()); }
        let mut doc = Document::load_mem(original).map_err(|e| format!("Failed to load PDF: {e:?}"))?;
        let pages_map = doc.get_pages();
        if pages_map.is_empty() { return Err("No pages".into()); }
        let mut ordered_ids = Vec::new();
        for n in 1..=pages_map.len() as u32 {
            if let Some(id) = pages_map.get(&n) { ordered_ids.push(*id); }
        }
        let mut keep_obj_ids = Vec::new();
        for &idx in keep_indices {
            if idx >= ordered_ids.len() { return Err(format!("Page {} out of range", idx + 1)); }
            keep_obj_ids.push(ordered_ids[idx]);
        }
        let catalog = doc.catalog().map_err(|e| format!("Catalog: {e:?}"))?;
        let pages_root_id = catalog.get(b"Pages").map_err(|e| format!("Pages: {e:?}"))?.as_reference().map_err(|e| format!("Pages ref: {e:?}"))?;
        let new_kids: Vec<Object> = keep_obj_ids.into_iter().map(Object::Reference).collect();
        if let Ok(dict) = doc.get_dictionary_mut(pages_root_id) {
            dict.set(b"Kids", Object::Array(new_kids));
            dict.set(b"Count", Object::Integer(keep_indices.len() as i64));
        }
        let mut out = Vec::new();
        { let mut cur = std::io::Cursor::new(&mut out); doc.save_to(&mut cur).map_err(|e| format!("Save: {e:?}"))?; }
        Ok(out)
    }

    pub fn print_pdf_vector(printer_name_opt: Option<&str>, pdf_bytes: &[u8], range: PrintRange, current_idx: usize, custom_input: &str, total_pages: usize) -> Result<String, String> {
        let keep_indices: Vec<usize> = match range {
            PrintRange::All => (0..total_pages).collect(),
            PrintRange::Current => vec![current_idx],
            PrintRange::Custom => parse_page_range(custom_input, total_pages)?,
        };
        let final_bytes = if keep_indices.len() == total_pages && matches!(range, PrintRange::All) {
            pdf_bytes.to_vec()
        } else {
            extract_pages_pdf(pdf_bytes, &keep_indices)?
        };
        let printer_name = if let Some(n) = printer_name_opt { n.to_string() } else { get_default_printer()? };
        let printer_w = to_wide(&printer_name);
        let doc_name_w = to_wide("PDF Document");
        let datatype_w = to_wide("RAW");
        unsafe {
            let mut hprinter: HANDLE = std::ptr::null_mut();
            let ok = OpenPrinterW(printer_w.as_ptr() as *mut u16, &mut hprinter as *mut _, std::ptr::null_mut());
            if ok == 0 { return Err(format!("Cannot open printer '{}'", printer_name)); }
            let mut doc_info = DOC_INFO_1W {
                pDocName: doc_name_w.as_ptr() as *mut u16,
                pOutputFile: std::ptr::null_mut(),
                pDatatype: datatype_w.as_ptr() as *mut u16,
            };
            let job_id = StartDocPrinterW(hprinter, 1, &mut doc_info as *mut _ as LPBYTE);
            if job_id == 0 { ClosePrinter(hprinter); return Err(format!("Failed to start job on '{}'", printer_name)); }
            if StartPagePrinter(hprinter) == 0 { EndDocPrinter(hprinter); ClosePrinter(hprinter); return Err("StartPage failed".into()); }
            let mut total = 0usize;
            const CHUNK: usize = 1024*1024;
            for chunk in final_bytes.chunks(CHUNK) {
                let mut written: DWORD = 0;
                let ok = WritePrinter(hprinter, chunk.as_ptr() as *mut _, chunk.len() as u32, &mut written);
                if ok == 0 { EndPagePrinter(hprinter); EndDocPrinter(hprinter); ClosePrinter(hprinter); return Err(format!("Write failed after {} bytes", total)); }
                total += written as usize;
            }
            if EndPagePrinter(hprinter) == 0 { EndDocPrinter(hprinter); ClosePrinter(hprinter); return Err("EndPage failed".into()); }
            if EndDocPrinter(hprinter) == 0 { ClosePrinter(hprinter); return Err("EndDoc failed".into()); }
            ClosePrinter(hprinter);
            Ok(format!("Sent to '{}' (Job {})", printer_name, job_id))
        }
    }
}

#[cfg(windows)]
use print_win::{enum_printers, get_default_printer, print_pdf_vector};
#[cfg(not(windows))]
fn enum_printers() -> Vec<String> { Vec::new() }
#[cfg(not(windows))]
fn get_default_printer() -> Result<String,String> { Err("Windows only".into()) }
#[cfg(not(windows))]
fn print_pdf_vector(_: Option<&str>, _: &[u8], _: PrintRange, _: usize, _: &str, _: usize) -> Result<String,String> { Err("Windows only".into()) }

// ───────────────────────────── app state ─────────────────────────────

struct ViewerApp {
    // document
    pdf_bytes: Option<Vec<u8>>,
    pdf_path: Option<PathBuf>,
    pdf_doc: Option<Pdf>,
    page_dims_cache: Vec<(f32, f32)>,
    file_size: Option<u64>,
    outline_items: Vec<outline::OutlineItem>,

    // view state
    page_idx: usize,
    total_pages: usize,
    scale: f32,
    fit_mode: FitMode,
    rotation: u8,
    continuous: bool,
    dark_mode: bool, // invert page colours
    hand_tool: bool,
    is_fullscreen: bool,

    // rendering / cache
    texture: Option<egui::TextureHandle>,
    last_render: Option<(usize, u32)>,
    tex_cache: HashMap<(usize, u32, u8, bool), egui::TextureHandle>,
    tex_lru: VecDeque<(usize, u32, u8, bool)>,
    tex_bytes: usize,
    renders_this_frame: usize,

    // messages
    error: Option<String>,
    status: Option<String>,
    status_since: Option<f64>,

    // page box in toolbar
    page_input: String,
    page_input_editing: bool,
    focus_page_input: bool,

    // dialogs / panels
    show_printer_picker: bool,
    available_printers: Vec<String>,
    selected_printer: Option<String>,
    print_range: PrintRange,
    custom_range_input: String,
    custom_range_error: Option<String>,
    show_password_dialog: bool,
    show_properties: bool,
    show_shortcuts: bool,
    show_outline: bool,

    // config
    config: AppConfig,
    config_path: PathBuf,
    config_dirty: bool,
    last_cfg_save: f64,
    last_title: String,

    // scrolling
    central_rect: egui::Rect,
    wheel_cooldown: f64,
    scroll_to_page: Option<usize>,
    scroll_to_offset: Option<f32>,
    continuous_last_offset: f32,
    single_viewport_h: f32,
    single_scroll_offset_target: Option<f32>,
    single_inner_offset_y: f32,
    single_inner_viewport_h: f32,
    single_inner_content_h: f32,
    single_inner_at_top: bool,
    single_inner_at_bottom: bool,
    single_inner_scroll_target: Option<f32>,
}

impl ViewerApp {
    fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.style_mut_of(egui::Theme::Dark, |s| apply_style(s, true));
        ctx.style_mut_of(egui::Theme::Light, |s| apply_style(s, false));
        // We bind Ctrl +/-/0 to *page* zoom, so stop egui from zooming the whole UI.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let cfg_path = config_path();
        let mut cfg = load_config(&cfg_path);
        if cfg.last_scale < MIN_SCALE {
            cfg.last_scale = 1.5;
        }
        apply_theme_pref(ctx, cfg.theme);

        let mut app = Self {
            pdf_bytes: None,
            pdf_path: None,
            pdf_doc: None,
            page_dims_cache: Vec::new(),
            file_size: None,
            outline_items: Vec::new(),

            page_idx: cfg.last_page,
            total_pages: 0,
            scale: cfg.last_scale,
            fit_mode: FitMode::None,
            rotation: 0,
            continuous: cfg.continuous,
            dark_mode: cfg.dark_mode,
            hand_tool: cfg.hand_tool,
            is_fullscreen: false,

            texture: None,
            last_render: None,
            tex_cache: HashMap::new(),
            tex_lru: VecDeque::new(),
            tex_bytes: 0,
            renders_this_frame: 0,

            error: None,
            status: None,
            status_since: None,

            page_input: "1".to_string(),
            page_input_editing: false,
            focus_page_input: false,

            show_printer_picker: false,
            available_printers: Vec::new(),
            selected_printer: None,
            print_range: PrintRange::All,
            custom_range_input: String::new(),
            custom_range_error: None,
            show_password_dialog: false,
            show_properties: false,
            show_shortcuts: false,
            show_outline: false,

            config: cfg.clone(),
            config_path: cfg_path,
            config_dirty: false,
            last_cfg_save: 0.0,
            last_title: String::new(),

            central_rect: egui::Rect::NOTHING,
            wheel_cooldown: 0.0,
            scroll_to_page: None,
            scroll_to_offset: None,
            continuous_last_offset: 0.0,
            single_viewport_h: 0.0,
            single_scroll_offset_target: None,
            single_inner_offset_y: 0.0,
            single_inner_viewport_h: 0.0,
            single_inner_content_h: 0.0,
            single_inner_at_top: true,
            single_inner_at_bottom: true,
            single_inner_scroll_target: None,
        };

        // 1. file from double-click / command line (added to Recent)
        // 2. otherwise restore the last session
        let explicit = initial_file.is_some();
        let file_to_open = initial_file.or_else(|| cfg.last_file.clone().map(PathBuf::from));
        if let Some(p) = file_to_open {
            if p.exists() {
                app.open_file_from_path(p, explicit);
            }
        }
        app
    }

    // ───────────── config / messages ─────────────

    fn save_config_now(&mut self) {
        self.config.last_page = self.page_idx;
        self.config.last_scale = self.scale;
        self.config.continuous = self.continuous;
        self.config.dark_mode = self.dark_mode;
        self.config.hand_tool = self.hand_tool;
        if let Some(p) = &self.pdf_path {
            let key = p.to_string_lossy().to_string();
            self.config.last_file = Some(key.clone());
            self.config.file_history.insert(
                key,
                FileHistory {
                    page: self.page_idx,
                    scale: self.scale,
                    continuous: self.continuous,
                    rotation: self.rotation,
                    dark_mode: self.dark_mode,
                },
            );
            if self.config.file_history.len() > 200 {
                let mut keep: std::collections::HashSet<String> =
                    self.config.recent.iter().cloned().collect();
                if let Some(last) = &self.config.last_file {
                    keep.insert(last.clone());
                }
                self.config.file_history.retain(|k, _| keep.contains(k));
            }
        }
        // Written to disk by `flush_config_if_due` (debounced) instead of on every scroll tick.
        self.config_dirty = true;
    }

    fn flush_config(&mut self, now: f64) {
        if self.config_dirty {
            save_config(&self.config, &self.config_path);
            self.config_dirty = false;
            self.last_cfg_save = now;
        }
    }

    fn flush_config_if_due(&mut self, ctx: &egui::Context, now: f64) {
        if !self.config_dirty {
            return;
        }
        if now - self.last_cfg_save >= CONFIG_SAVE_INTERVAL {
            self.flush_config(now);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
        self.status_since = None;
    }

    fn push_recent(&mut self, path: &Path) {
        let s = path.to_string_lossy().to_string();
        self.config.recent.retain(|x| x != &s);
        self.config.recent.insert(0, s);
        self.config.recent.truncate(12);
    }

    // ───────────── open / close ─────────────

    fn open_file_from_path(&mut self, path: PathBuf, save_recent: bool) {
        let key = path.to_string_lossy().to_string();
        let hist = self.config.file_history.get(&key).cloned();
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(format!("Cannot read file: {e}"));
                return;
            }
        };
        match Pdf::new(bytes.clone()) {
            Ok(pdf) => {
                let dims: Vec<(f32, f32)> =
                    pdf.pages().iter().map(|p| p.render_dimensions()).collect();
                self.total_pages = dims.len();
                self.page_dims_cache = dims;
                self.pdf_doc = Some(pdf);
                self.file_size = Some(bytes.len() as u64);
                self.outline_items = outline::extract_outline(&bytes);
                self.pdf_bytes = Some(bytes);
                self.pdf_path = Some(path.clone());

                if let Some(h) = hist.as_ref() {
                    self.page_idx = h.page.min(self.total_pages.saturating_sub(1));
                    self.scale = if h.scale > 0.05 { quantize_scale(h.scale).max(MIN_SCALE) } else { 1.0 };
                    self.continuous = h.continuous;
                    self.rotation = h.rotation % 4;
                    self.dark_mode = h.dark_mode;
                    self.fit_mode = FitMode::None;
                } else if !save_recent {
                    // restoring the last session: page/scale already come from the config
                    self.fit_mode = FitMode::None;
                } else {
                    // brand-new file: start on page 1, fitted to the window width
                    self.page_idx = 0;
                    self.scale = 1.0;
                    self.rotation = 0;
                    self.fit_mode = FitMode::Width;
                }
                if self.page_idx >= self.total_pages {
                    self.page_idx = 0;
                }
                self.page_input = (self.page_idx + 1).to_string();
                self.clear_tex_cache();
                self.error = None;
                self.status = None;
                self.show_password_dialog = false;
                self.outline_items_changed();
                if save_recent {
                    self.push_recent(&path);
                }
                self.config.last_file = Some(key);
                self.save_config_now();
                // Both modes need an explicit scroll target to land on the restored page.
                self.scroll_to_page = Some(self.page_idx);
                self.single_inner_scroll_target = Some(0.0);
                self.single_inner_offset_y = 0.0;
                self.single_inner_at_top = true;
                self.set_status(format!("Opened {}", file_name_of(&path)));
            }
            Err(e) => {
                let msg = format!("{:?}", e).to_lowercase();
                if msg.contains("encrypt") || msg.contains("password") || msg.contains("decrypt") {
                    self.show_password_dialog = true;
                    self.error = None;
                } else {
                    self.error = Some(format!("Failed to open PDF: {e:?}"));
                }
            }
        }
    }

    fn outline_items_changed(&mut self) {
        // The outline panel is opt-in; keep its previous visibility, but hide it
        // when a document without bookmarks is opened.
        if self.outline_items.is_empty() {
            self.show_outline = false;
        }
    }

    fn open_file_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
            self.open_file_from_path(path, true);
        }
    }

    fn close_file(&mut self) {
        self.pdf_bytes = None;
        self.pdf_path = None;
        self.pdf_doc = None;
        self.page_dims_cache.clear();
        self.total_pages = 0;
        self.page_idx = 0;
        self.page_input = "1".to_string();
        self.clear_tex_cache();
        self.error = None;
        self.status = None;
        self.rotation = 0;
        self.outline_items.clear();
        self.show_outline = false;
        self.show_properties = false;
        self.config.last_file = None;
        self.file_size = None;
        self.config_dirty = true;
    }

    // ───────────── texture cache ─────────────

    fn clear_tex_cache(&mut self) {
        self.tex_cache.clear();
        self.tex_lru.clear();
        self.tex_bytes = 0;
        self.texture = None;
        self.last_render = None;
    }

    /// Returns the (cached) texture for a page. With `defer_ok`, at most
    /// `MAX_RENDERS_PER_FRAME` new pages are rendered per frame so fast scrolling
    /// in continuous mode never freezes the UI; the caller draws a placeholder.
    fn get_or_render_texture(
        &mut self,
        page_idx: usize,
        scale: f32,
        ctx: &egui::Context,
        defer_ok: bool,
    ) -> Option<egui::TextureHandle> {
        let bits = scale_bits(scale);
        let rot = self.rotation;
        let dark = self.dark_mode;
        let key = (page_idx, bits, rot, dark);
        if let Some(tex) = self.tex_cache.get(&key) {
            let tex = tex.clone();
            if let Some(pos) = self.tex_lru.iter().position(|k| k == &key) {
                self.tex_lru.remove(pos);
                self.tex_lru.push_back(key);
            }
            return Some(tex);
        }
        if defer_ok && self.renders_this_frame >= MAX_RENDERS_PER_FRAME {
            ctx.request_repaint();
            return None;
        }
        self.renders_this_frame += 1;

        let pdf = self.pdf_doc.as_ref()?;
        if page_idx >= pdf.pages().len() {
            return None;
        }
        let page = &pdf.pages()[page_idx];
        let cache = RenderCache::new();
        let (pw, ph) = self.page_dims_cache.get(page_idx).copied().unwrap_or((612.0, 792.0));
        let mut s = quantize_scale(scale);
        const MAX_PIX: usize = 24_000_000;
        let pix_w = (pw * s) as usize;
        let pix_h = (ph * s) as usize;
        if pix_w * pix_h > MAX_PIX {
            let ratio = (MAX_PIX as f32 / (pix_w * pix_h) as f32).sqrt();
            s *= ratio;
        }
        let pixmap = hayro::render(
            page,
            &cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings { x_scale: s, y_scale: s, bg_color: WHITE },
        );
        let w = pixmap.width() as usize;
        let h = pixmap.height() as usize;
        if w == 0 || h == 0 {
            return None;
        }
        let bytes_slice = bytemuck::cast_slice(pixmap.data());
        let (mut rotated_bytes, rw, rh) = if rot != 0 {
            Self::rotate_pixmap(bytes_slice, w, h, rot)
        } else {
            (bytes_slice.to_vec(), w, h)
        };
        if dark {
            Self::invert_rgba(&mut rotated_bytes);
        }
        let color_image = egui::ColorImage::from_rgba_premultiplied([rw, rh], &rotated_bytes);
        let tex = ctx.load_texture(
            format!("p{}-{}-{}-{}-q", page_idx, bits, rot, if dark { 1 } else { 0 }),
            color_image,
            egui::TextureOptions::LINEAR,
        );

        // evict by count *and* by memory budget
        let new_bytes = rw * rh * 4;
        while !self.tex_lru.is_empty()
            && (self.tex_cache.len() >= MAX_TEX_COUNT || self.tex_bytes + new_bytes > MAX_TEX_BYTES)
        {
            if let Some(old) = self.tex_lru.pop_front() {
                if let Some(t) = self.tex_cache.remove(&old) {
                    let [tw, th] = t.size();
                    self.tex_bytes = self.tex_bytes.saturating_sub(tw * th * 4);
                }
            }
        }
        self.tex_bytes += new_bytes;
        self.tex_cache.insert(key, tex.clone());
        self.tex_lru.push_back(key);
        Some(tex)
    }

    fn render_single_page(&mut self, ctx: &egui::Context) {
        if self.pdf_doc.is_none() {
            return;
        }
        let bits = scale_bits(self.scale);
        if let Some((p, s)) = self.last_render {
            if p == self.page_idx && s == bits && self.texture.is_some() {
                return;
            }
        }
        if let Some(tex) = self.get_or_render_texture(self.page_idx, self.scale, ctx, false) {
            self.texture = Some(tex);
            self.last_render = Some((self.page_idx, bits));
        }
    }

    // ───────────── navigation ─────────────

    fn bottom_offset(&self, idx: usize) -> f32 {
        let q = quantize_scale(self.scale);
        self.rotated_dims(idx)
            .map(|(_, h)| (h * q - self.single_inner_viewport_h).max(0.0))
            .unwrap_or(0.0)
    }

    /// Single entry point for every page change (buttons, keys, outline, wheel).
    fn go_to_ex(&mut self, idx: usize, land_bottom: bool) {
        if self.total_pages == 0 {
            return;
        }
        let idx = idx.min(self.total_pages - 1);
        self.page_idx = idx;
        self.page_input = (idx + 1).to_string();
        self.texture = None;
        self.save_config_now();
        if self.continuous {
            self.scroll_to_page = Some(idx);
        } else {
            if self.single_viewport_h > 1.0 {
                self.single_scroll_offset_target = Some(idx as f32 * self.single_viewport_h);
            } else {
                self.scroll_to_page = Some(idx);
            }
            let target = if land_bottom { self.bottom_offset(idx) } else { 0.0 };
            self.single_inner_offset_y = target;
            self.single_inner_scroll_target = Some(target);
            self.single_inner_at_top = target <= 0.0;
            self.single_inner_at_bottom = land_bottom;
        }
    }

    fn go_to(&mut self, idx: usize) {
        self.go_to_ex(idx, false);
    }
    fn jump_to_page(&mut self, page_1_based: usize) {
        if page_1_based >= 1 && page_1_based <= self.total_pages {
            self.go_to(page_1_based - 1);
        }
    }
    fn first_page(&mut self) {
        self.go_to(0);
    }
    fn last_page(&mut self) {
        if self.total_pages > 0 {
            self.go_to(self.total_pages - 1);
        }
    }
    fn next_page(&mut self) {
        if self.page_idx + 1 < self.total_pages {
            self.go_to(self.page_idx + 1);
        }
    }
    fn prev_page(&mut self) {
        if self.page_idx > 0 {
            self.go_to(self.page_idx - 1);
        }
    }

    // ───────────── zoom / view ─────────────

    fn rotated_dims(&self, idx: usize) -> Option<(f32, f32)> {
        let (w, h) = self.page_dims_cache.get(idx).copied()?;
        if self.rotation % 2 == 1 { Some((h, w)) } else { Some((w, h)) }
    }

    fn doc_height(&self, s: f32) -> f32 {
        (0..self.page_dims_cache.len())
            .filter_map(|i| self.rotated_dims(i))
            .map(|(_, h)| h * s + ROW_PAD)
            .sum()
    }

    /// The one place the zoom level changes. Keeps the reading position in
    /// continuous mode by rescaling the scroll offset.
    fn set_scale(&mut self, new_scale: f32) {
        let qs = quantize_scale(new_scale.clamp(MIN_SCALE, MAX_SCALE));
        if (qs - self.scale).abs() < 0.001 {
            return;
        }
        if self.continuous && self.pdf_doc.is_some() {
            let (h_old, h_new) = (self.doc_height(quantize_scale(self.scale)), self.doc_height(qs));
            if h_old > 0.0 {
                let off = self.continuous_last_offset * h_new / h_old;
                self.scroll_to_offset = Some(off);
                self.continuous_last_offset = off;
            }
        }
        self.scale = qs;
        self.texture = None;
        self.save_config_now();
    }

    fn zoom_in(&mut self) {
        self.fit_mode = FitMode::None;
        self.set_scale(step_zoom(self.scale, true));
    }
    fn zoom_out(&mut self) {
        self.fit_mode = FitMode::None;
        self.set_scale(step_zoom(self.scale, false));
    }
    fn zoom_reset(&mut self) {
        self.fit_mode = FitMode::None;
        self.set_scale(1.0);
    }
    fn zoom_to(&mut self, s: f32) {
        self.fit_mode = FitMode::None;
        self.set_scale(s);
    }
    fn fit_width(&mut self) {
        self.fit_mode = FitMode::Width;
        self.texture = None;
    }
    fn fit_height(&mut self) {
        self.fit_mode = FitMode::Height;
        self.texture = None;
    }

    fn rotate_cw(&mut self) {
        self.rotation = (self.rotation + 1) % 4;
        self.clear_tex_cache();
        self.save_config_now();
    }
    fn rotate_ccw(&mut self) {
        self.rotation = (self.rotation + 3) % 4;
        self.clear_tex_cache();
        self.save_config_now();
    }
    fn toggle_dark_mode(&mut self) {
        self.dark_mode = !self.dark_mode;
        self.clear_tex_cache();
        self.save_config_now();
    }
    fn toggle_hand_tool(&mut self) {
        self.hand_tool = !self.hand_tool;
        self.save_config_now();
    }
    fn set_continuous(&mut self, on: bool) {
        if on == self.continuous {
            return;
        }
        self.continuous = on;
        self.scroll_to_page = Some(self.page_idx);
        if !on {
            self.single_inner_scroll_target = Some(0.0);
        }
        self.save_config_now();
    }
    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.is_fullscreen = !self.is_fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.is_fullscreen));
    }

    fn open_print_dialog(&mut self) {
        self.available_printers = enum_printers();
        if self.available_printers.is_empty() {
            if let Ok(def) = get_default_printer() {
                self.available_printers = vec![def.clone()];
                self.selected_printer = Some(def);
            }
        } else if self.selected_printer.is_none() {
            if let Ok(def) = get_default_printer() {
                self.selected_printer = Some(def);
            } else {
                self.selected_printer = self.available_printers.first().cloned();
            }
        }
        self.show_printer_picker = true;
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = match &self.pdf_path {
            Some(p) => format!("{} — {}", file_name_of(p), APP_NAME),
            None => APP_NAME.to_string(),
        };
        // only talk to the window manager when something actually changed
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn invert_rgba(data: &mut [u8]) {
        // Fast invert: unrolled for speed
        for chunk in data.chunks_exact_mut(4) {
            chunk[0] = 255 - chunk[0];
            chunk[1] = 255 - chunk[1];
            chunk[2] = 255 - chunk[2];
            // alpha keep
        }
    }

    fn rotate_pixmap(data: &[u8], w: usize, h: usize, rot: u8) -> (Vec<u8>, usize, usize) {
        // data is RGBA, len = w*h*4
        // Fast path for 0
        if rot % 4 == 0 {
            return (data.to_vec(), w, h);
        }
        let rot = rot % 4;
        match rot {
            1 => {
                // 90 CW: new_w = h, new_h = w
                let new_w = h;
                let new_h = w;
                let mut out = vec![0u8; data.len()];
                // For speed, process 4 bytes at a time
                for y in 0..h {
                    for x in 0..w {
                        let src_idx = (y * w + x) * 4;
                        // dst: (h-1-y, x) in new coords
                        let dst_x = h - 1 - y;
                        let dst_y = x;
                        let dst_idx = (dst_y * new_w + dst_x) * 4;
                        out[dst_idx..dst_idx+4].copy_from_slice(&data[src_idx..src_idx+4]);
                    }
                }
                (out, new_w, new_h)
            }
            2 => {
                // 180
                let mut out = vec![0u8; data.len()];
                for y in 0..h {
                    for x in 0..w {
                        let src_idx = (y * w + x) * 4;
                        let dst_x = w - 1 - x;
                        let dst_y = h - 1 - y;
                        let dst_idx = (dst_y * w + dst_x) * 4;
                        out[dst_idx..dst_idx+4].copy_from_slice(&data[src_idx..src_idx+4]);
                    }
                }
                (out, w, h)
            }
            3 => {
                // 270 CW (90 CCW)
                let new_w = h;
                let new_h = w;
                let mut out = vec![0u8; data.len()];
                for y in 0..h {
                    for x in 0..w {
                        let src_idx = (y * w + x) * 4;
                        let dst_x = y;
                        let dst_y = w - 1 - x;
                        let dst_idx = (dst_y * new_w + dst_x) * 4;
                        out[dst_idx..dst_idx+4].copy_from_slice(&data[src_idx..src_idx+4]);
                    }
                }
                (out, new_w, new_h)
            }
            _ => (data.to_vec(), w, h),
        }
    }
}

// ───────────────────────────── UI helpers ─────────────────────────────

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+O", "Open file"),
    ("Ctrl+W", "Close file"),
    ("Ctrl+P", "Print"),
    ("Ctrl+I", "Document properties"),
    ("Ctrl+Q", "Quit"),
    ("←  →  ·  PageUp  PageDown", "Previous / next page"),
    ("↑  ↓", "Scroll (single-page mode jumps to the next page at the edge)"),
    ("Space  /  Shift+Space", "Next / previous screen"),
    ("Home  /  End", "First / last page"),
    ("Ctrl+G", "Go to page"),
    ("+  −  ·  Ctrl+Wheel", "Zoom in / out"),
    ("0", "Actual size (100%)"),
    ("W  /  H", "Fit width / fit height"),
    ("R  /  Shift+R", "Rotate clockwise / counter-clockwise"),
    ("D", "Invert page colours"),
    ("C", "Toggle continuous scrolling"),
    ("Alt+H", "Hand tool (drag to pan)"),
    ("F9  /  Ctrl+B", "Show / hide outline"),
    ("F11", "Full screen"),
    ("F1", "Keyboard shortcuts"),
    ("Esc", "Close dialog / leave full screen"),
];

fn tb_btn(ui: &mut egui::Ui, text: &str, tip: &str) -> egui::Response {
    ui.add(egui::Button::new(text).min_size(vec2(30.0, 28.0))).on_hover_text(tip)
}

fn tb_toggle(ui: &mut egui::Ui, text: &str, tip: &str, on: bool) -> egui::Response {
    ui.add(egui::Button::new(text).selected(on).min_size(vec2(30.0, 28.0)))
        .on_hover_text(tip)
}

fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    let mut b = egui::Button::new(label);
    if !shortcut.is_empty() {
        b = b.shortcut_text(shortcut);
    }
    let clicked = ui.add_enabled(enabled, b).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn outline_label(
    ui: &mut egui::Ui,
    title: &str,
    target: Option<usize>,
    active: bool,
    jump: &mut Option<usize>,
) {
    let mut text = RichText::new(title);
    if target.is_none() {
        text = text.weak();
    }
    let mut resp = ui.selectable_label(active, text);
    if let Some(p) = target {
        resp = resp.on_hover_text(format!("Page {}", p + 1));
        if resp.clicked() {
            *jump = Some(p);
        }
    }
}

fn outline_node(
    ui: &mut egui::Ui,
    item: &outline::OutlineItem,
    id: egui::Id,
    current: usize,
    jump: &mut Option<usize>,
) {
    let target = item.page_idx.or_else(|| item.children.iter().find_map(|c| c.page_idx));
    let title = {
        let t = item.title.trim();
        if t.is_empty() { "(untitled)" } else { t }
    };
    let active = item.page_idx == Some(current);
    if item.children.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            outline_label(ui, title, target, active, jump);
        });
    } else {
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        state
            .show_header(ui, |ui| {
                outline_label(ui, title, target, active, jump);
            })
            .body(|ui| {
                for (i, child) in item.children.iter().enumerate() {
                    outline_node(ui, child, id.with(i), current, jump);
                }
            });
    }
}

// ───────────────────────────── input handling ─────────────────────────────

impl ViewerApp {
    fn modal_open(&self) -> bool {
        self.show_printer_picker || self.show_properties || self.show_password_dialog || self.show_shortcuts
    }

    fn pointer_over_canvas(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| i.pointer.hover_pos()).map_or(false, |p| self.central_rect.contains(p))
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            if let Some(path) = file.path {
                if path.extension().map_or(false, |e| e.eq_ignore_ascii_case("pdf")) {
                    self.open_file_from_path(path, true);
                    return;
                } else {
                    self.error = Some("Only PDF files can be opened.".to_string());
                }
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::Key;
        let has_pdf = self.pdf_doc.is_some();
        let wants_text = ctx.wants_keyboard_input();
        let modal = self.modal_open();
        let (cmd, shift, alt) =
            ctx.input(|i| (i.modifiers.command, i.modifiers.shift, i.modifiers.alt));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));

        // Esc closes the top-most thing
        if pressed(Key::Escape) {
            if self.show_printer_picker {
                self.show_printer_picker = false;
            } else if self.show_password_dialog {
                self.show_password_dialog = false;
            } else if self.show_properties {
                self.show_properties = false;
            } else if self.show_shortcuts {
                self.show_shortcuts = false;
            } else if self.is_fullscreen {
                self.toggle_fullscreen(ctx);
            } else {
                self.error = None;
            }
        }
        if pressed(Key::F11) {
            self.toggle_fullscreen(ctx);
        }
        if pressed(Key::F1) {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if cmd && pressed(Key::Q) {
            self.flush_config(ctx.input(|i| i.time));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if cmd && has_pdf && pressed(Key::I) {
            self.show_properties = !self.show_properties;
        }
        if (pressed(Key::F9) || (cmd && !shift && pressed(Key::B))) && !self.outline_items.is_empty() {
            self.show_outline = !self.show_outline;
        }

        // Ctrl-chords (single handler per action; no double triggering)
        if cmd && !modal {
            if pressed(Key::O) {
                self.open_file_dialog();
            }
            if has_pdf {
                if pressed(Key::P) {
                    self.open_print_dialog();
                }
                if pressed(Key::W) {
                    self.close_file();
                    return;
                }
                if pressed(Key::G) {
                    self.focus_page_input = true;
                }
                if pressed(Key::R) {
                    if shift { self.rotate_ccw() } else { self.rotate_cw() }
                }
                if pressed(Key::D) {
                    self.toggle_dark_mode();
                }
                if pressed(Key::Plus) || pressed(Key::Equals) {
                    self.zoom_in();
                }
                if pressed(Key::Minus) {
                    self.zoom_out();
                }
                if pressed(Key::Num0) {
                    self.zoom_reset();
                }
                if pressed(Key::Num8) {
                    self.fit_width();
                }
                if pressed(Key::Num9) {
                    self.fit_height();
                }
            }
            return;
        }

        // Plain keys (never while typing in a text field or with a dialog open)
        if has_pdf && !wants_text && !modal {
            if alt && !cmd {
                if pressed(Key::H) {
                    self.toggle_hand_tool();
                }
                return;
            }
            if pressed(Key::Plus) || pressed(Key::Equals) {
                self.zoom_in();
            }
            if pressed(Key::Minus) {
                self.zoom_out();
            }
            if pressed(Key::Num0) {
                self.zoom_reset();
            }
            if pressed(Key::W) {
                self.fit_width();
            }
            if pressed(Key::H) {
                self.fit_height();
            }
            if pressed(Key::R) {
                if shift { self.rotate_ccw() } else { self.rotate_cw() }
            }
            if pressed(Key::D) {
                self.toggle_dark_mode();
            }
            if pressed(Key::C) {
                self.set_continuous(!self.continuous);
            }
            if pressed(Key::Home) {
                self.first_page();
            }
            if pressed(Key::End) {
                self.last_page();
            }
        }
    }

    /// Ctrl+Wheel zoom (only over the document, never while a dialog is open).
    fn handle_zoom_wheel(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.modifiers.command) {
            return;
        }
        let wheel_y = ctx.input(|i| i.raw_scroll_delta.y);
        if self.pdf_doc.is_some() && !self.modal_open() && self.pointer_over_canvas(ctx) && wheel_y.abs() > 0.1 {
            self.fit_mode = FitMode::None;
            let up = wheel_y > 0.0;
            let factor = if up { 1.12 } else { 1.0 / 1.12 };
            let mut ns = quantize_scale((self.scale * factor).clamp(MIN_SCALE, MAX_SCALE));
            if (ns - self.scale).abs() < 0.001 {
                // never get "stuck" on the 5% render grid
                ns = quantize_scale(self.scale + if up { 0.05 } else { -0.05 });
            }
            self.set_scale(ns);
        }
        // swallow ALL scroll while Ctrl is held
        ctx.input_mut(|i| {
            i.raw_scroll_delta = egui::Vec2::ZERO;
            i.smooth_scroll_delta = egui::Vec2::ZERO;
        });
    }

    /// One edge-aware step in single-page mode: scroll inside the page first,
    /// change page only when the edge is reached.
    fn single_step(&mut self, dir: i32, step: f32) {
        let fits = self.single_inner_content_h <= self.single_inner_viewport_h + 2.0
            || self.single_inner_content_h < 1.0;
        if dir > 0 {
            if self.single_inner_at_bottom || fits {
                self.next_page();
            } else {
                let max_off = (self.single_inner_content_h - self.single_inner_viewport_h).max(0.0);
                self.single_inner_scroll_target = Some((self.single_inner_offset_y + step).min(max_off));
            }
        } else if self.single_inner_at_top || fits {
            if self.page_idx > 0 {
                self.go_to_ex(self.page_idx - 1, true); // land at the bottom of the previous page
            }
        } else {
            self.single_inner_scroll_target = Some((self.single_inner_offset_y - step).max(0.0));
        }
    }

    fn handle_navigation(&mut self, ctx: &egui::Context, now: f64) {
        use egui::Key;
        if self.pdf_doc.is_none() || self.modal_open() {
            return;
        }
        let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        if cmd {
            return;
        }
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));

        if !ctx.wants_keyboard_input() {
            if pressed(Key::ArrowRight) || pressed(Key::PageDown) {
                self.next_page();
            }
            if pressed(Key::ArrowLeft) || pressed(Key::PageUp) {
                self.prev_page();
            }
            if self.continuous {
                let mut delta = 0.0;
                if pressed(Key::ArrowDown) {
                    delta += 90.0;
                }
                if pressed(Key::ArrowUp) {
                    delta -= 90.0;
                }
                if delta != 0.0 {
                    let off = (self.continuous_last_offset + delta).max(0.0);
                    self.scroll_to_offset = Some(off);
                    self.continuous_last_offset = off;
                }
                if pressed(Key::Space) {
                    if shift { self.prev_page() } else { self.next_page() }
                }
            } else {
                if pressed(Key::ArrowDown) {
                    self.single_step(1, 120.0);
                }
                if pressed(Key::ArrowUp) {
                    self.single_step(-1, 120.0);
                }
                if pressed(Key::Space) {
                    let step = (self.single_inner_viewport_h * 0.9).max(120.0);
                    self.single_step(if shift { -1 } else { 1 }, step);
                }
            }
        }

        // Wheel at the page edge flips pages (single-page mode, only over the document)
        if !self.continuous && self.pointer_over_canvas(ctx) && now - self.wheel_cooldown > 0.18 {
            let wheel = ctx.input(|i| i.raw_scroll_delta.y);
            let fits = self.single_inner_content_h <= self.single_inner_viewport_h + 2.0
                || self.single_inner_content_h < 1.0;
            if wheel < -1.0 && (self.single_inner_at_bottom || fits) {
                if self.page_idx + 1 < self.total_pages {
                    self.next_page();
                    self.wheel_cooldown = now;
                }
            } else if wheel > 1.0 && (self.single_inner_at_top || fits) && self.page_idx > 0 {
                self.go_to_ex(self.page_idx - 1, true);
                self.wheel_cooldown = now;
            }
        }
    }
}

// ───────────────────────────── chrome: menu, toolbar, status ─────────────────────────────

impl ViewerApp {
    fn ui_top_panels(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(8, 2)))
            .show(ctx, |ui| self.ui_menu_row(ui, ctx));
        egui::TopBottomPanel::top("toolbar")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(10, 6)))
            .show(ctx, |ui| self.ui_toolbar(ui));
    }

    fn ui_menu_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let has = self.pdf_doc.is_some();
        let recents = self.config.recent.clone();
        ui.horizontal(|ui| {
            // flat, borderless menu titles
            ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;

            ui.menu_button("File", |ui| {
                ui.set_min_width(230.0);
                if menu_item(ui, "Open…", "Ctrl+O", true) {
                    self.open_file_dialog();
                }
                ui.menu_button("Open recent", |ui| {
                    ui.set_min_width(300.0);
                    if recents.is_empty() {
                        ui.label(RichText::new("No recent files").weak());
                    } else {
                        for r in &recents {
                            let name = file_name_of(Path::new(r));
                            if ui.button(name).on_hover_text(r.as_str()).clicked() {
                                self.open_file_from_path(PathBuf::from(r), true);
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button("Clear recent files").clicked() {
                            self.config.recent.clear();
                            self.config_dirty = true;
                            ui.close();
                        }
                    }
                });
                ui.separator();
                if menu_item(ui, "Print…", "Ctrl+P", has) {
                    self.open_print_dialog();
                }
                if menu_item(ui, "Properties…", "Ctrl+I", has) {
                    self.show_properties = true;
                }
                ui.separator();
                if menu_item(ui, "Close file", "Ctrl+W", has) {
                    self.close_file();
                }
                if menu_item(ui, "Quit", "Ctrl+Q", true) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            ui.menu_button("View", |ui| {
                ui.set_min_width(250.0);
                let mut cont = self.continuous;
                if ui.checkbox(&mut cont, "Continuous scroll").changed() {
                    self.set_continuous(cont);
                }
                let mut inv = self.dark_mode;
                if ui.add_enabled(has, egui::Checkbox::new(&mut inv, "Invert page colours")).changed() {
                    self.toggle_dark_mode();
                }
                let mut hand = self.hand_tool;
                if ui.add_enabled(has, egui::Checkbox::new(&mut hand, "Hand tool")).changed() {
                    self.toggle_hand_tool();
                }
                let mut ol = self.show_outline;
                let has_outline = !self.outline_items.is_empty();
                if ui.add_enabled(has_outline, egui::Checkbox::new(&mut ol, "Outline")).changed() {
                    self.show_outline = ol;
                }
                ui.separator();
                if menu_item(ui, "Zoom in", "Ctrl++", has) {
                    self.zoom_in();
                }
                if menu_item(ui, "Zoom out", "Ctrl+−", has) {
                    self.zoom_out();
                }
                if menu_item(ui, "Actual size", "Ctrl+0", has) {
                    self.zoom_reset();
                }
                if menu_item(ui, "Fit width", "W", has) {
                    self.fit_width();
                }
                if menu_item(ui, "Fit height", "H", has) {
                    self.fit_height();
                }
                ui.separator();
                if menu_item(ui, "Rotate clockwise", "R", has) {
                    self.rotate_cw();
                }
                if menu_item(ui, "Rotate counter-clockwise", "Shift+R", has) {
                    self.rotate_ccw();
                }
                ui.separator();
                ui.menu_button("Theme", |ui| {
                    for (label, p) in [
                        ("System", ThemePref::System),
                        ("Light", ThemePref::Light),
                        ("Dark", ThemePref::Dark),
                    ] {
                        if ui.radio(self.config.theme == p, label).clicked() {
                            self.config.theme = p;
                            apply_theme_pref(ctx, p);
                            self.config_dirty = true;
                            ui.close();
                        }
                    }
                });
                let fs_label = if self.is_fullscreen { "Exit full screen" } else { "Full screen" };
                if menu_item(ui, fs_label, "F11", true) {
                    self.toggle_fullscreen(ctx);
                }
            });

            ui.menu_button("Help", |ui| {
                ui.set_min_width(220.0);
                if menu_item(ui, "Keyboard shortcuts", "F1", true) {
                    self.show_shortcuts = true;
                }
                ui.separator();
                ui.label(RichText::new(format!("{} · hayro renderer", APP_NAME)).small().weak());
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(p) = &self.pdf_path {
                    ui.label(RichText::new(file_name_of(p)).small().weak());
                }
            });
        });
    }

    fn ui_toolbar(&mut self, ui: &mut egui::Ui) {
        let has = self.pdf_doc.is_some();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;

            if ui
                .add(egui::Button::new("📂  Open").min_size(vec2(0.0, 28.0)))
                .on_hover_text("Open a PDF (Ctrl+O)")
                .clicked()
            {
                self.open_file_dialog();
            }
            ui.add_enabled_ui(has, |ui| {
                if tb_btn(ui, "🖨", "Print (Ctrl+P)").clicked() {
                    self.open_print_dialog();
                }
                if tb_btn(ui, "ℹ", "Document properties (Ctrl+I)").clicked() {
                    self.show_properties = !self.show_properties;
                }
            });

            ui.separator();

            // ── page navigation (works in both view modes) ──
            ui.add_enabled_ui(has, |ui| {
                if tb_btn(ui, "‹", "Previous page (←)").clicked() {
                    self.prev_page();
                }
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.page_input)
                        .desired_width(42.0)
                        .horizontal_align(egui::Align::Center),
                );
                if self.focus_page_input {
                    resp.request_focus();
                    self.focus_page_input = false;
                }
                self.page_input_editing = resp.has_focus();
                if resp.lost_focus() {
                    match self.page_input.trim().parse::<usize>() {
                        Ok(n) if n >= 1 && n <= self.total_pages => self.jump_to_page(n),
                        _ => self.page_input = (self.page_idx + 1).to_string(),
                    }
                }
                ui.label(RichText::new(format!("/ {}", self.total_pages.max(1))).weak());
                if tb_btn(ui, "›", "Next page (→)").clicked() {
                    self.next_page();
                }
            });

            ui.separator();

            // ── zoom ──
            ui.add_enabled_ui(has, |ui| {
                if tb_btn(ui, "−", "Zoom out (Ctrl+−)").clicked() {
                    self.zoom_out();
                }
                let zoom_label = format!("{:.0}%", self.scale * 100.0);
                ui.menu_button(zoom_label, |ui| {
                    ui.set_min_width(130.0);
                    for z in [0.5_f32, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0] {
                        let selected = self.fit_mode == FitMode::None && (self.scale - z).abs() < 0.001;
                        if ui.selectable_label(selected, format!("{:.0}%", z * 100.0)).clicked() {
                            self.zoom_to(z);
                            ui.close();
                        }
                    }
                });
                if tb_btn(ui, "+", "Zoom in (Ctrl++)").clicked() {
                    self.zoom_in();
                }
                let fw = self.fit_mode == FitMode::Width;
                if tb_toggle(ui, "Fit W", "Fit to width (W)", fw).clicked() {
                    self.fit_width();
                }
                let fh = self.fit_mode == FitMode::Height;
                if tb_toggle(ui, "Fit H", "Fit to height (H)", fh).clicked() {
                    self.fit_height();
                }
            });

            ui.separator();

            // ── rotate ──
            ui.add_enabled_ui(has, |ui| {
                if tb_btn(ui, "↺", "Rotate counter-clockwise (Shift+R)").clicked() {
                    self.rotate_ccw();
                }
                if tb_btn(ui, "↻", "Rotate clockwise (R)").clicked() {
                    self.rotate_cw();
                }
            });

            ui.separator();

            // ── view options ──
            ui.add_enabled_ui(has, |ui| {
                let cont = self.continuous;
                if tb_toggle(ui, "Continuous", "Continuous scrolling (C)", cont).clicked() {
                    self.set_continuous(!cont);
                }
                let hand = self.hand_tool;
                if tb_toggle(ui, "✋ Hand", "Hand tool — drag to pan (Alt+H)", hand).clicked() {
                    self.toggle_hand_tool();
                }
                let inv = self.dark_mode;
                if tb_toggle(ui, "🌙 Invert", "Invert page colours (D)", inv).clicked() {
                    self.toggle_dark_mode();
                }
            });
            ui.add_enabled_ui(!self.outline_items.is_empty(), |ui| {
                let on = self.show_outline;
                if tb_toggle(ui, "📑 Outline", "Show / hide outline (F9)", on).clicked() {
                    self.show_outline = !on;
                }
            });
        });
    }

    fn ui_status_bar(&mut self, ctx: &egui::Context) {
        let dark = ctx.style().visuals.dark_mode;
        let ok_color = if dark { Color32::from_rgb(120, 205, 140) } else { Color32::from_rgb(22, 128, 60) };
        egui::TopBottomPanel::bottom("status_bar")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(12, 4)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    match &self.status {
                        Some(s) => {
                            ui.label(RichText::new(format!("✔ {}", s)).small().color(ok_color));
                        }
                        None => {
                            ui.label(RichText::new("Ready").small().weak());
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.pdf_doc.is_none() {
                            return;
                        }
                        let mut parts: Vec<String> = vec![
                            format!("Page {} of {}", self.page_idx + 1, self.total_pages),
                            format!("{:.0}%", self.scale * 100.0),
                        ];
                        if self.rotation != 0 {
                            parts.push(format!("Rotated {}°", self.rotation as u32 * 90));
                        }
                        if self.dark_mode {
                            parts.push("Inverted".to_string());
                        }
                        parts.push(format_file_size(self.file_size));
                        // right-to-left layout: add the last item first
                        for (i, p) in parts.iter().rev().enumerate() {
                            if i > 0 {
                                ui.separator();
                            }
                            ui.label(RichText::new(p).small());
                        }
                    });
                });
            });
    }

    fn ui_error_banner(&mut self, ctx: &egui::Context) {
        let Some(err) = self.error.clone() else {
            return;
        };
        let dark = ctx.style().visuals.dark_mode;
        let (bg, fg) = if dark {
            (Color32::from_rgb(78, 34, 38), Color32::from_rgb(255, 205, 205))
        } else {
            (Color32::from_rgb(254, 226, 226), Color32::from_rgb(153, 27, 27))
        };
        let mut dismiss = false;
        egui::TopBottomPanel::top("error_banner")
            .frame(egui::Frame::new().fill(bg).inner_margin(egui::Margin::symmetric(14, 8)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").color(fg));
                    ui.label(RichText::new(&err).color(fg));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Dismiss (Esc)").clicked() {
                            dismiss = true;
                        }
                    });
                });
            });
        if dismiss {
            self.error = None;
        }
    }

    fn ui_outline_panel(&mut self, ctx: &egui::Context) {
        if !self.show_outline || self.outline_items.is_empty() || self.is_fullscreen {
            return;
        }
        let items = std::mem::take(&mut self.outline_items);
        let current = self.page_idx;
        let mut jump: Option<usize> = None;
        let mut close = false;
        egui::SidePanel::left("outline_panel")
            .resizable(true)
            .default_width(260.0)
            .min_width(160.0)
            .max_width(480.0)
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(10, 8)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Outline").strong().size(15.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Close (F9)").clicked() {
                            close = true;
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (i, item) in items.iter().enumerate() {
                        outline_node(ui, item, egui::Id::new("outline_node").with(i), current, &mut jump);
                    }
                });
            });
        self.outline_items = items;
        if close {
            self.show_outline = false;
        }
        if let Some(p) = jump {
            self.go_to(p);
        }
    }
}

// ───────────────────────────── document canvas ─────────────────────────────

const UV_FULL: egui::Rect = egui::Rect {
    min: egui::Pos2 { x: 0.0, y: 0.0 },
    max: egui::Pos2 { x: 1.0, y: 1.0 },
};

impl ViewerApp {
    fn ui_central(&mut self, ctx: &egui::Context) {
        let dark_ui = ctx.style().visuals.dark_mode;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(canvas_color(dark_ui)))
            .show(ctx, |ui| {
                self.central_rect = ui.max_rect();
                if self.pdf_doc.is_none() {
                    self.ui_welcome(ui);
                    return;
                }

                // Fit W / Fit H: recomputed every frame so it follows window resizes
                if self.fit_mode != FitMode::None {
                    if let Some((pw, ph)) = self.rotated_dims(self.page_idx) {
                        let avail = ui.available_size();
                        let ns = match self.fit_mode {
                            FitMode::Width => (avail.x - 2.0 * PAGE_MARGIN_X) / pw,
                            FitMode::Height => {
                                (avail.y - if self.continuous { ROW_PAD + 8.0 } else { 16.0 }) / ph
                            }
                            FitMode::None => self.scale,
                        };
                        if ns.is_finite() {
                            self.set_scale(ns.clamp(MIN_SCALE, MAX_SCALE));
                        }
                    }
                }

                if self.continuous {
                    self.ui_continuous(ui, ctx, dark_ui);
                } else {
                    self.ui_single(ui, ctx, dark_ui);
                }

                if self.hand_tool && !self.modal_open() && self.pointer_over_canvas(ctx) {
                    ctx.set_cursor_icon(if ctx.input(|i| i.pointer.is_decidedly_dragging()) {
                        egui::CursorIcon::Grabbing
                    } else {
                        egui::CursorIcon::Grab
                    });
                }
            });
    }

    fn ui_welcome(&mut self, ui: &mut egui::Ui) {
        let avail_h = ui.available_height();
        let recents: Vec<String> = self.config.recent.iter().take(8).cloned().collect();
        let mut to_open: Option<String> = None;
        let mut open_dialog = false;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((avail_h * 0.14).clamp(16.0, 140.0));
                ui.label(RichText::new("📄").size(56.0));
                ui.add_space(4.0);
                ui.label(RichText::new(APP_NAME).size(28.0).strong());
                ui.label(RichText::new("Fast, lightweight PDF viewing").weak());
                ui.add_space(20.0);
                let open_btn = egui::Button::new(RichText::new("📂  Open PDF…").size(16.0).color(Color32::WHITE))
                    .fill(ACCENT_DARK)
                    .min_size(vec2(200.0, 42.0));
                if ui.add(open_btn).clicked() {
                    open_dialog = true;
                }
                ui.add_space(6.0);
                ui.label(RichText::new("or drop a file anywhere in this window  ·  Ctrl+O").small().weak());

                if !recents.is_empty() {
                    ui.add_space(34.0);
                    ui.label(RichText::new("Recent files").strong());
                    ui.add_space(6.0);
                    ui.allocate_ui_with_layout(
                        vec2(520.0, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            for r in &recents {
                                let name = file_name_of(Path::new(r));
                                let dir = parent_of(r);
                                let btn = egui::Button::new(RichText::new(format!("📄  {}", name))).frame(false);
                                if ui.add(btn).on_hover_text(r.as_str()).clicked() {
                                    to_open = Some(r.clone());
                                }
                                ui.label(RichText::new(dir).small().weak());
                                ui.add_space(4.0);
                            }
                        },
                    );
                }
                ui.add_space(30.0);
                ui.label(RichText::new("Press F1 for keyboard shortcuts").small().weak());
            });
        });

        if open_dialog {
            self.open_file_dialog();
        }
        if let Some(p) = to_open {
            self.open_file_from_path(PathBuf::from(p), true);
        }
    }

    fn ui_continuous(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, dark_ui: bool) {
        let q = quantize_scale(self.scale);
        let total = self.total_pages;
        if total == 0 {
            return;
        }
        // Every row has an exact, predictable height, so virtual scrolling never drifts.
        let row_heights: Vec<f32> = (0..total)
            .map(|i| self.rotated_dims(i).map(|(_, h)| h * q + ROW_PAD).unwrap_or(ROW_PAD))
            .collect();
        let total_h: f32 = row_heights.iter().sum();
        let max_w = (0..total)
            .filter_map(|i| self.rotated_dims(i).map(|(w, _)| w * q))
            .fold(0.0_f32, f32::max);

        let mut target_offset: Option<f32> = None;
        if let Some(page) = self.scroll_to_page.take() {
            target_offset = Some(row_heights[..page.min(total)].iter().sum::<f32>());
        }
        if let Some(off) = self.scroll_to_offset.take() {
            target_offset = Some(off);
        }

        let mut area = egui::ScrollArea::both().auto_shrink([false; 2]).scroll_source(
            egui::containers::scroll_area::ScrollSource {
                scroll_bar: true,
                drag: self.hand_tool,
                mouse_wheel: true,
            },
        );
        if let Some(off) = target_offset {
            area = area.vertical_scroll_offset(off);
        }

        let label_color = ui.visuals().weak_text_color();
        let placeholder = if self.dark_mode { Color32::from_gray(38) } else { Color32::from_gray(250) };
        let mut new_page: Option<usize> = None;

        area.show_viewport(ui, |ui, viewport| {
            self.continuous_last_offset = viewport.min.y;
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let content_w = (max_w + 2.0 * PAGE_MARGIN_X).max(viewport.width());
            ui.set_min_size(vec2(content_w, total_h));

            // visible window (+ a margin so pages are ready before they scroll in)
            let top = viewport.min.y - 300.0;
            let bottom = viewport.max.y + 300.0;
            let mut cum = 0.0_f32;
            let mut first = 0usize;
            let mut first_set = false;
            let mut last = total - 1;
            for (i, h) in row_heights.iter().enumerate() {
                let next = cum + *h;
                if !first_set && next >= top {
                    first = i;
                    first_set = true;
                }
                if cum > bottom {
                    last = i.saturating_sub(1);
                    break;
                }
                cum = next;
            }

            // current page = the page under the upper part of the viewport
            let ref_y = viewport.min.y + viewport.height() * 0.3;
            let mut center_page = self.page_idx.min(total - 1);
            let mut acc = 0.0_f32;
            for (i, h) in row_heights.iter().enumerate() {
                if ref_y >= acc && ref_y < acc + *h {
                    center_page = i;
                    break;
                }
                acc += *h;
            }
            if viewport.min.y <= 1.0 {
                center_page = 0;
            } else if viewport.max.y >= total_h - 1.0 {
                center_page = total - 1;
            }
            if center_page != self.page_idx {
                new_page = Some(center_page);
            }

            first = first.saturating_sub(1);
            last = (last + 1).min(total - 1);

            let before_h: f32 = row_heights[..first].iter().sum();
            if before_h > 0.0 {
                ui.add_space(before_h);
            }
            for idx in first..=last {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(content_w, row_heights[idx]), egui::Sense::hover());
                let tex = self.get_or_render_texture(idx, q, ctx, true);
                let (pw, ph) = self
                    .rotated_dims(idx)
                    .map(|(w, h)| (w * q, h * q))
                    .unwrap_or((0.0, 0.0));
                let size = tex.as_ref().map(|t| t.size_vec2()).unwrap_or(vec2(pw, ph));
                let page_rect = egui::Rect::from_min_size(
                    pos2((rect.center().x - size.x / 2.0).round(), (rect.top() + PAGE_TOP).round()),
                    size,
                );
                let painter = ui.painter();
                painter.extend(page_shadow(page_rect, dark_ui));
                match &tex {
                    Some(t) => {
                        painter.image(t.id(), page_rect, UV_FULL, Color32::WHITE);
                    }
                    None => {
                        painter.rect_filled(page_rect, 0.0, placeholder);
                        painter.text(
                            page_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Rendering…",
                            egui::FontId::proportional(14.0),
                            Color32::GRAY,
                        );
                    }
                }
                painter.text(
                    pos2(page_rect.center().x, rect.top() + 7.0),
                    egui::Align2::CENTER_TOP,
                    format!("Page {}", idx + 1),
                    egui::FontId::proportional(11.5),
                    label_color,
                );
            }
            let after_h: f32 = row_heights[(last + 1)..].iter().sum();
            if after_h > 0.0 {
                ui.add_space(after_h);
            }
        });

        if let Some(p) = new_page {
            if p != self.page_idx {
                self.page_idx = p;
                if !self.page_input_editing {
                    self.page_input = (p + 1).to_string();
                }
                self.save_config_now();
            }
        }
    }

    fn ui_single(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, dark_ui: bool) {
        self.render_single_page(ctx);
        let Some(tex) = self.texture.clone() else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Rendering…").weak());
            });
            return;
        };
        let total = self.total_pages.max(1);

        // A pending page jump can only be turned into an offset once the viewport height is known.
        let mut target_offset: Option<f32> = None;
        if self.single_viewport_h > 1.0 {
            if let Some(page) = self.scroll_to_page.take() {
                target_offset = Some(page as f32 * self.single_viewport_h);
            }
        } else if self.scroll_to_page.is_some() {
            ctx.request_repaint();
        }
        if let Some(off) = self.single_scroll_offset_target.take() {
            target_offset = Some(off);
        }

        let full_rect = ui.available_rect_before_wrap();
        let bar_w = 16.0;
        let main_rect = egui::Rect::from_min_max(
            full_rect.min,
            pos2(full_rect.max.x - bar_w - 2.0, full_rect.max.y),
        );
        let scroll_rect = egui::Rect::from_min_max(pos2(full_rect.max.x - bar_w, full_rect.min.y), full_rect.max);

        let mut new_page_from_scroll: Option<usize> = None;
        let mut re_anchored = false;
        let hand = self.hand_tool;
        let q = quantize_scale(self.scale);

        // Main view: scrolls inside the page first (edge tracking for page changes)
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(main_rect), |ui| {
            let mut inner = egui::ScrollArea::both()
                .id_salt(format!("single_pan_{}", self.page_idx))
                .auto_shrink([false, false])
                .scroll_source(egui::containers::scroll_area::ScrollSource {
                    scroll_bar: false,
                    drag: hand,
                    mouse_wheel: true,
                })
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
            if let Some(off) = self.single_inner_scroll_target.take() {
                inner = inner.vertical_scroll_offset(off);
            }
            inner.show_viewport(ui, |ui, viewport| {
                self.single_inner_offset_y = viewport.min.y;
                self.single_inner_viewport_h = viewport.height();
                let mut content_h = viewport.height();
                if let Some((_, ph)) = self.rotated_dims(self.page_idx) {
                    content_h = (ph * q).max(viewport.height());
                }
                self.single_inner_content_h = content_h;
                let eps = 2.0;
                self.single_inner_at_top = viewport.min.y <= eps;
                self.single_inner_at_bottom = viewport.min.y + viewport.height() >= content_h - eps;

                ui.centered_and_justified(|ui| {
                    let shadow_idx = ui.painter().add(egui::Shape::Noop);
                    let resp = ui.image(&tex);
                    ui.painter().set(shadow_idx, egui::Shape::Vec(page_shadow(resp.rect, dark_ui)));
                });
            });
        });

        // Right-hand page scrollbar: dragging it changes the page
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(scroll_rect), |ui| {
            let mut outer = egui::ScrollArea::vertical()
                .id_salt("single_page_nav_bar")
                .auto_shrink([false, false])
                .scroll_source(egui::containers::scroll_area::ScrollSource {
                    scroll_bar: true,
                    drag: false,
                    mouse_wheel: false,
                })
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
            if let Some(off) = target_offset {
                outer = outer.vertical_scroll_offset(off);
            }
            outer.show_viewport(ui, |ui, viewport| {
                let vp_h = viewport.height().max(1.0);
                let prev_h = self.single_viewport_h;
                self.single_viewport_h = vp_h;
                if (prev_h - vp_h).abs() > 0.5 {
                    // first measurement or window resized: re-anchor on the current page
                    // instead of re-deriving (and silently changing) the page from the offset
                    self.single_scroll_offset_target = Some(self.page_idx as f32 * vp_h);
                    re_anchored = true;
                } else {
                    let page = ((viewport.min.y / vp_h).round() as usize).min(total - 1);
                    if page != self.page_idx {
                        new_page_from_scroll = Some(page);
                    }
                }
                ui.add_space(vp_h * total as f32);
            });
        });

        if re_anchored {
            ctx.request_repaint();
        }
        if let Some(new_page) = new_page_from_scroll {
            if new_page != self.page_idx {
                self.page_idx = new_page;
                self.page_input = (new_page + 1).to_string();
                self.texture = None;
                self.save_config_now();
                self.single_inner_offset_y = 0.0;
                self.single_inner_scroll_target = Some(0.0);
                self.single_inner_at_top = true;
                self.single_inner_at_bottom = false;
            }
        }
    }
}

// ───────────────────────────── dialogs ─────────────────────────────

impl ViewerApp {
    fn dialog_window<'a>(title: &str) -> egui::Window<'a> {
        egui::Window::new(RichText::new(title).strong())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
    }

    fn ui_dialogs(&mut self, ctx: &egui::Context) {
        self.ui_password_dialog(ctx);
        self.ui_print_dialog(ctx);
        self.ui_properties_dialog(ctx);
        self.ui_shortcuts_dialog(ctx);
    }

    fn ui_password_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_password_dialog {
            return;
        }
        let mut open = true;
        let mut close = false;
        Self::dialog_window("Encrypted PDF").open(&mut open).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.label("This PDF is password-protected.");
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "The rendering engine does not support encrypted documents yet. \
                     Remove the password with another tool, then open the file again.",
                )
                .weak(),
            );
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("Close").min_size(vec2(90.0, 28.0))).clicked() {
                    close = true;
                }
            });
        });
        if close || !open {
            self.show_password_dialog = false;
        }
    }

    fn ui_print_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_printer_picker {
            return;
        }
        // validate the custom range once per frame
        self.custom_range_error = if self.print_range == PrintRange::Custom {
            parse_page_range(&self.custom_range_input, self.total_pages).err()
        } else {
            None
        };

        let mut open = true;
        let mut do_print = false;
        let mut cancel = false;
        Self::dialog_window("Print").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(400.0);

            ui.label(RichText::new("Printer").strong());
            if self.available_printers.is_empty() {
                ui.label(RichText::new("No printers found.").weak());
                if ui.button("Refresh").clicked() {
                    self.available_printers = enum_printers();
                    if let Ok(def) = get_default_printer() {
                        self.selected_printer = Some(def);
                    }
                }
            } else {
                egui::ComboBox::from_id_salt("printer_combo")
                    .width(380.0)
                    .selected_text(self.selected_printer.clone().unwrap_or_else(|| "Select printer".into()))
                    .show_ui(ui, |ui| {
                        for name in &self.available_printers {
                            ui.selectable_value(&mut self.selected_printer, Some(name.clone()), name);
                        }
                    });
            }

            ui.add_space(10.0);
            ui.label(RichText::new("Pages").strong());
            ui.radio_value(&mut self.print_range, PrintRange::All, format!("All ({} pages)", self.total_pages));
            ui.radio_value(
                &mut self.print_range,
                PrintRange::Current,
                format!("Current page ({})", self.page_idx + 1),
            );
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.print_range, PrintRange::Custom, "Custom:");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.custom_range_input)
                        .hint_text("e.g. 1,3,5-7")
                        .desired_width(180.0),
                );
                if self.print_range != PrintRange::Custom && resp.gained_focus() {
                    self.print_range = PrintRange::Custom;
                }
            });
            if self.print_range == PrintRange::Custom && !self.custom_range_input.trim().is_empty() {
                if let Some(err) = &self.custom_range_error {
                    let c = if ui.visuals().dark_mode {
                        Color32::from_rgb(255, 130, 130)
                    } else {
                        Color32::from_rgb(190, 30, 30)
                    };
                    ui.label(RichText::new(err).small().color(c));
                }
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(4.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_print = self.selected_printer.is_some()
                    && self.pdf_bytes.is_some()
                    && match self.print_range {
                        PrintRange::Custom => {
                            !self.custom_range_input.trim().is_empty() && self.custom_range_error.is_none()
                        }
                        _ => true,
                    };
                let print_btn = egui::Button::new(RichText::new("Print").color(Color32::WHITE))
                    .fill(ACCENT_DARK)
                    .min_size(vec2(90.0, 28.0));
                if ui.add_enabled(can_print, print_btn).clicked() {
                    do_print = true;
                }
                if ui.add(egui::Button::new("Cancel").min_size(vec2(90.0, 28.0))).clicked() {
                    cancel = true;
                }
            });
        });

        if do_print {
            if let Some(bytes) = self.pdf_bytes.clone() {
                let sel = self.selected_printer.clone();
                match print_pdf_vector(
                    sel.as_deref(),
                    &bytes,
                    self.print_range,
                    self.page_idx,
                    &self.custom_range_input,
                    self.total_pages,
                ) {
                    Ok(msg) => {
                        self.set_status(msg);
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(e);
                        self.status = None;
                    }
                }
            }
            self.show_printer_picker = false;
        }
        if cancel || !open {
            self.show_printer_picker = false;
        }
    }

    fn ui_properties_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_properties {
            return;
        }
        let mut open = true;
        let mut close = false;
        Self::dialog_window("Document properties").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(440.0);
            match &self.pdf_path {
                Some(path) => {
                    let full = path.to_string_lossy().to_string();
                    egui::Grid::new("props_grid").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
                        ui.label(RichText::new("File").weak());
                        ui.label(file_name_of(path));
                        ui.end_row();

                        ui.label(RichText::new("Location").weak());
                        ui.horizontal_wrapped(|ui| {
                            ui.label(&full);
                        });
                        ui.end_row();

                        ui.label(RichText::new("Size").weak());
                        ui.label(format_file_size(self.file_size));
                        ui.end_row();

                        ui.label(RichText::new("Pages").weak());
                        ui.label(self.total_pages.to_string());
                        ui.end_row();

                        if let Some((w, h)) = self.rotated_dims(self.page_idx) {
                            ui.label(RichText::new("Page size").weak());
                            ui.label(format!("{:.0} × {:.0} pt  ({:.1} × {:.1} mm)", w, h, w * 0.3528, h * 0.3528));
                            ui.end_row();
                        }

                        ui.label(RichText::new("Zoom").weak());
                        ui.label(format!("{:.0}%", self.scale * 100.0));
                        ui.end_row();

                        ui.label(RichText::new("Current page").weak());
                        ui.label(format!("{} / {}", self.page_idx + 1, self.total_pages));
                        ui.end_row();
                    });
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Copy path").clicked() {
                            ui.ctx().copy_text(full.clone());
                        }
                        if ui.button("Show in folder").clicked() {
                            if let Some(parent) = path.parent() {
                                #[cfg(windows)]
                                let _ = std::process::Command::new("explorer").arg(parent).spawn();
                                #[cfg(not(windows))]
                                let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("Close").min_size(vec2(90.0, 28.0))).clicked() {
                                close = true;
                            }
                        });
                    });
                }
                None => {
                    ui.label("No file open");
                }
            }
        });
        if close || !open {
            self.show_properties = false;
        }
    }

    fn ui_shortcuts_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut open = true;
        let mut close = false;
        Self::dialog_window("Keyboard shortcuts").open(&mut open).show(ctx, |ui| {
            egui::Grid::new("shortcuts_grid").num_columns(2).spacing([24.0, 7.0]).striped(true).show(ui, |ui| {
                for (keys, what) in SHORTCUTS {
                    ui.label(RichText::new(*keys).monospace().strong());
                    ui.label(*what);
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("Close").min_size(vec2(90.0, 28.0))).clicked() {
                    close = true;
                }
            });
        });
        if close || !open {
            self.show_shortcuts = false;
        }
    }

    fn draw_drop_overlay(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("file_drop_overlay"),
        ));
        let rect = ctx.screen_rect();
        painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(12, 18, 32, 205));
        painter.text(
            rect.center() - vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            "Drop a PDF to open it",
            egui::FontId::proportional(28.0),
            Color32::WHITE,
        );
        painter.text(
            rect.center() + vec2(0.0, 26.0),
            egui::Align2::CENTER_CENTER,
            "Release the mouse button anywhere in this window",
            egui::FontId::proportional(14.0),
            Color32::from_gray(190),
        );
    }
}

// ───────────────────────────── eframe glue ─────────────────────────────

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        self.renders_this_frame = 0;

        self.handle_dropped_files(ctx);
        self.handle_shortcuts(ctx);
        self.handle_zoom_wheel(ctx);
        self.handle_navigation(ctx, now);
        self.update_window_title(ctx);

        // status messages fade out on their own
        if self.status.is_some() {
            match self.status_since {
                None => self.status_since = Some(now),
                Some(t) if now - t > STATUS_LIFETIME => {
                    self.status = None;
                    self.status_since = None;
                }
                _ => {}
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }

        if !self.is_fullscreen {
            self.ui_top_panels(ctx);
        }
        self.ui_error_banner(ctx);
        if !self.is_fullscreen {
            self.ui_status_bar(ctx);
        }
        self.ui_outline_panel(ctx);
        self.ui_central(ctx);
        self.ui_dialogs(ctx);
        self.draw_drop_overlay(ctx);

        if ctx.input(|i| i.viewport().close_requested()) {
            self.flush_config(now);
        } else {
            self.flush_config_if_due(ctx, now);
        }
    }
}

fn main() -> eframe::Result<()> {
    // Windows file association / double-click passes the PDF path as first argument
    let initial_file = cli_pdf_path_from_args();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([720.0, 440.0])
            .with_title(APP_NAME)
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(ViewerApp::new(cc, initial_file)))),
    )
}
