//! SVG to PNG with resvg, and SVG with text turned into paths (spec 8.1).
//!
//! Only the bundled Inter is available: no system fonts, so a render looks
//! the same on every machine. resvg is built without raster-image support,
//! and `resources_dir` is unset, so an SVG can't pull in other files.

use std::sync::{Arc, OnceLock};

use resvg::tiny_skia;
use resvg::usvg;

use super::text::{INTER_BOLD, INTER_REGULAR, INTER_SEMIBOLD};
use crate::error::{AppError, AppResult};

/// Largest PNG we'll make, in pixels.
pub const MAX_PIXELS: f64 = 40_000_000.0;
/// Largest drawing, in canvas units on either side.
pub const MAX_SIDE: f64 = 20_000.0;

fn options() -> &'static usvg::Options<'static> {
    static OPTIONS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        for font in [INTER_REGULAR, INTER_SEMIBOLD, INTER_BOLD] {
            db.load_font_data(font.to_vec());
        }
        db.set_sans_serif_family("Inter");
        usvg::Options {
            resources_dir: None,
            font_family: "Inter".into(),
            fontdb: Arc::new(db),
            ..usvg::Options::default()
        }
    })
}

fn parse(svg: &str) -> AppResult<usvg::Tree> {
    let tree = usvg::Tree::from_str(svg, options())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("rendering failed: {e}")))?;
    let size = tree.size();
    if f64::from(size.width()) > MAX_SIDE || f64::from(size.height()) > MAX_SIDE {
        return Err(AppError::State {
            code: "too_large",
            message: format!("that's larger than {MAX_SIDE} units across; export a smaller frame"),
        });
    }
    Ok(tree)
}

/// Rasterise at `scale` (1 = one pixel per canvas unit).
pub fn to_png(svg: &str, scale: f32) -> AppResult<Vec<u8>> {
    let tree = parse(svg)?;
    let size = tree.size();
    let (w, h) = (
        (size.width() * scale).ceil(),
        (size.height() * scale).ceil(),
    );
    if f64::from(w) * f64::from(h) > MAX_PIXELS {
        return Err(AppError::State {
            code: "too_large",
            message: "that image would be over 40 megapixels; use a smaller scale".into(),
        });
    }
    let mut pixmap = tiny_skia::Pixmap::new(w as u32, h as u32)
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("empty image")))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("encoding PNG: {e}")))
}

/// The same SVG with its text converted to paths, so it looks identical
/// anywhere, whether or not Inter is installed.
pub fn text_to_paths(svg: &str) -> AppResult<String> {
    let tree = parse(svg)?;
    Ok(tree.to_string(&usvg::WriteOptions::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" font-family="Inter"><rect width="200" height="100" fill="#fff"/><text x="10" y="50" font-size="20">Hello</text></svg>"##;

    #[test]
    fn makes_a_png_at_the_right_size() {
        let png = to_png(SVG, 2.0).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR: width and height, big-endian, at bytes 16..24.
        let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((w, h), (400, 200));
    }

    #[test]
    fn text_becomes_paths() {
        let out = text_to_paths(SVG).unwrap();
        assert!(!out.contains("<text"), "{out}");
        assert!(out.contains("<path"), "glyphs drawn as paths");
    }

    #[test]
    fn refuses_huge_renders() {
        let huge = r#"<svg xmlns="http://www.w3.org/2000/svg" width="19000" height="19000"></svg>"#;
        assert!(matches!(to_png(huge, 1.0), Err(AppError::State { .. })));
        let too_wide =
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="25000" height="10"></svg>"#;
        assert!(to_png(too_wide, 1.0).is_err());
    }
}
