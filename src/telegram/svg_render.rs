use std::sync::Arc;
use std::sync::OnceLock;
use tiny_skia::Pixmap;
use usvg::{fontdb, Options, Tree};

static FONT_DB: OnceLock<Arc<fontdb::Database>> = OnceLock::new();

fn get_font_db() -> Arc<fontdb::Database> {
    FONT_DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        let font_dir = std::path::Path::new("assets/fonts");
        if font_dir.exists() {
            db.load_fonts_dir(font_dir);
        }
        db.load_system_fonts();
        db.set_sans_serif_family("Vazirmatn");
        Arc::new(db)
    }).clone()
}

pub fn fmt_bytes(n: i64) -> String {
    if n <= 0 {
        return "0 B".to_string();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut x = n as f64;
    let mut u = 0;
    while x >= 1024.0 && u < units.len() - 1 {
        x /= 1024.0;
        u += 1;
    }
    if x >= 100.0 {
        format!("{:.0} {}", x, units[u])
    } else if x >= 10.0 {
        format!("{:.1} {}", x, units[u])
    } else {
        format!("{:.2} {}", x, units[u])
    }
}

pub fn render_svg_to_png(svg: &str, scale: f32) -> Result<Vec<u8>, String> {
    let fontdb = get_font_db();
    let mut opt = Options::default();
    opt.font_family = "Vazirmatn".to_string();
    opt.fontdb = fontdb;

    let tree = Tree::from_str(svg, &opt).map_err(|e| format!("SVG parse error: {}", e))?;
    let pixmap_size = tree.size();
    let width = (pixmap_size.width() * scale).ceil() as u32;
    let height = (pixmap_size.height() * scale).ceil() as u32;

    let mut pixmap = Pixmap::new(width.max(1), height.max(1))
        .ok_or_else(|| "Failed to allocate pixmap".to_string())?;

    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    pixmap.encode_png().map_err(|e| format!("PNG encode error: {}", e))
}
