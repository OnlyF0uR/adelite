//! Screens drawn offscreen and saved as PNGs, to look at the design in
//! every scheme without opening the app. For tests.

use std::path::Path;

use crate::gpu::Offscreen;
use crate::image::{self, ImageAtlas};
use crate::paint::{Painter, Rect};
use crate::text::{Fonts, GlyphAtlas};
use crate::theme::Scheme;
use crate::ui::Ui;

/// Draws a screen of `size` logical pixels with `draw` until its animations
/// settled and its pictures are decoded, then renders the last frame as
/// RGBA pixels at a scale of one.
pub fn frame(
    gpu: &mut Offscreen,
    fonts: &Fonts,
    scheme: Scheme,
    size: (u32, u32),
    draw: &mut dyn FnMut(&mut Painter<'_>, &mut Ui, Rect),
) -> Vec<u8> {
    let (mut atlas, mut images) = (GlyphAtlas::default(), ImageAtlas::default());
    let mut ui = Ui::default();
    (ui.focused, ui.mouse) = (true, (-1.0, -1.0));
    let view = Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32);
    let palette = scheme.palette();
    let mut instances = Vec::new();
    for _ in 0..5 {
        // A long frame: every ease lands at once.
        ui.begin(1.0, 10.0);
        let mut p = Painter::new(fonts, (&mut atlas, &mut images), &mut instances, 1.0, view, palette);
        draw(&mut p, &mut ui, view);
        ui.end();
        for key in images.take_wanted() {
            let pixels = image::thumbnail(&key);
            images.insert(key, pixels);
        }
    }
    gpu.render(size, &instances, (&mut atlas.uploads, &mut images.uploads), 1.0, palette.panel)
}

/// Writes RGBA `pixels` as a PNG.
///
/// # Panics
/// The file cannot be written: a snapshot run has nothing to show then.
pub fn write_png(path: &Path, (width, height): (u32, u32), pixels: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).expect("snapshot file"));
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("PNG header");
    writer.write_image_data(pixels).expect("PNG data");
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::logo::Logo;
    use crate::paint::hex;

    /// Draws the app's icon (its mark on a dark tile) at every size the
    /// platforms want, into the folder `ADELITE_ICON` names:
    /// `ADELITE_ICON=<workspace>/crates/desktop/assets/icon cargo test -p adelite-ui --features offscreen icon -- --ignored`
    /// It also draws the logo for use outside the app into `branding/` at
    /// the workspace's root: square to its corners, as whoever shows it rounds it.
    #[test]
    #[ignore = "draws on the GPU; run by hand to make the icon again"]
    fn icon() {
        let Some(dir) = std::env::var_os("ADELITE_ICON").map(PathBuf::from) else { return };
        let Some(mut gpu) = Offscreen::new() else { return };
        let fonts = Fonts::load();
        let branding = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../branding");
        std::fs::create_dir_all(&branding).expect("branding folder");
        std::fs::create_dir_all(&dir).expect("icon folder");
        let sizes = [16u32, 20, 24, 32, 40, 48, 64, 256, 1024].map(|size| (size, false));
        for (size, square) in sizes.into_iter().chain([(500, true), (2000, true)]) {
            let (mut atlas, mut images) = (GlyphAtlas::default(), ImageAtlas::default());
            let side = size as f32;
            let view = Rect::new(0.0, 0.0, side, side);
            let mut instances = Vec::new();
            let palette = Scheme::Black.palette();
            let mut p = Painter::new(&fonts, (&mut atlas, &mut images), &mut instances, 1.0, view, palette);
            // macOS wants air around the tile; the others fill the square.
            let inset = if size == 1024 { side * 0.1 } else { 0.0 };
            let tile = Rect::new(inset, inset, side - 2.0 * inset, side - 2.0 * inset);
            // The square one keeps its mark clear of a round crop.
            let (radius, mark) = if square { (0.0, tile.w * 0.7) } else { (tile.w * 0.225, tile.w * 0.82) };
            p.gradient(tile, hex(0x26262B), hex(0x0C0C0E), radius);
            p.logo(Logo::App, (side * 0.5, side * 0.5), mark, hex(0xF2EEE6));
            let pixels = gpu.render((size, size), &instances, (&mut atlas.uploads, &mut images.uploads), 1.0, [0.0; 4]);
            let path = match size {
                _ if square => branding.join(format!("adelite-{size}.png")),
                1024 => dir.join("macos.png"),
                _ => dir.join(format!("{size}.png")),
            };
            write_png(&path, (size, size), &pixels);
        }
    }
}
