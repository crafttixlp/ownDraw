//! Rasterformate außer PNG: JPEG, WebP, AVIF, TIFF, BMP, GIF sowie animiertes GIF und APNG.

use image::codecs::gif::{GifEncoder, Repeat};
use image::{ExtendedColorType, Frame, ImageEncoder, RgbImage, RgbaImage};
use tiny_skia::Pixmap;

/// Zielformat für [`encode`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Raster {
    /// Qualität 1..100.
    Jpeg { quality: u8 },
    Webp,
    Avif,
    Tiff,
    Bmp,
    Gif,
}

/// Demultipliziertes RGBA-Bild.
fn rgba(pm: &Pixmap) -> RgbaImage {
    let mut buf = Vec::with_capacity(pm.pixels().len() * 4);
    for p in pm.pixels() {
        let c = p.demultiply();
        buf.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    RgbaImage::from_raw(pm.width(), pm.height(), buf).expect("Pixelpuffer passt")
}

/// Bild deckend auf `bg` (0xAARRGGBB, Alpha ignoriert) gelegt; arbeitet direkt auf vormultiplizierten Werten.
fn on_bg(pm: &Pixmap, bg: u32) -> RgbImage {
    let (br, bgc, bb) = ((bg >> 16) & 0xff, (bg >> 8) & 0xff, bg & 0xff);
    let mut buf = Vec::with_capacity(pm.pixels().len() * 3);
    for p in pm.pixels() {
        let inv = 255 - p.alpha() as u32;
        let mix = |c: u8, b: u32| (c as u32 + (b * inv + 127) / 255).min(255) as u8;
        buf.extend_from_slice(&[mix(p.red(), br), mix(p.green(), bgc), mix(p.blue(), bb)]);
    }
    RgbImage::from_raw(pm.width(), pm.height(), buf).expect("Pixelpuffer passt")
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Kodiert das (vormultiplizierte) Bild. Formate ohne Transparenz (JPEG, BMP) werden auf `bg` (0xAARRGGBB, deckend) gelegt.
pub fn encode(pm: &Pixmap, kind: Raster, bg: u32) -> Result<Vec<u8>, String> {
    let (w, h) = (pm.width(), pm.height());
    let mut out = Vec::new();
    match kind {
        Raster::Jpeg { quality } => {
            let img = on_bg(pm, bg);
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
                .write_image(img.as_raw(), w, h, ExtendedColorType::Rgb8)
                .map_err(err)?;
        }
        Raster::Webp => {
            image::codecs::webp::WebPEncoder::new_lossless(&mut out)
                .write_image(rgba(pm).as_raw(), w, h, ExtendedColorType::Rgba8)
                .map_err(err)?;
        }
        Raster::Avif => {
            // Geschwindigkeit 8 (von 1..10): Export soll nicht minutenlang dauern.
            image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut out, 8, 85)
                .write_image(rgba(pm).as_raw(), w, h, ExtendedColorType::Rgba8)
                .map_err(err)?;
        }
        Raster::Tiff => {
            let mut cur = std::io::Cursor::new(Vec::new());
            image::codecs::tiff::TiffEncoder::new(&mut cur)
                .write_image(rgba(pm).as_raw(), w, h, ExtendedColorType::Rgba8)
                .map_err(err)?;
            out = cur.into_inner();
        }
        Raster::Bmp => {
            image::codecs::bmp::BmpEncoder::new(&mut out)
                .write_image(on_bg(pm, bg).as_raw(), w, h, ExtendedColorType::Rgb8)
                .map_err(err)?;
        }
        Raster::Gif => {
            let mut enc = GifEncoder::new_with_speed(&mut out, 10);
            enc.encode_frame(Frame::new(rgba(pm))).map_err(err)?;
            drop(enc);
        }
    }
    Ok(out)
}

/// Gemeinsame Prüfung: mindestens ein Bild, alle gleich groß.
fn check_frames(frames: &[Pixmap]) -> Result<(u32, u32), String> {
    let first = frames.first().ok_or("keine Bilder")?;
    let (w, h) = (first.width(), first.height());
    if frames.iter().any(|f| f.width() != w || f.height() != h) {
        return Err("Bilder haben unterschiedliche Größen".into());
    }
    Ok((w, h))
}

/// Animiertes GIF (Endlosschleife), jedes Bild `delay_ms` lang; alle Bilder gleich groß.
pub fn gif_anim(frames: &[Pixmap], delay_ms: u32) -> Result<Vec<u8>, String> {
    check_frames(frames)?;
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new_with_speed(&mut out, 10);
        enc.set_repeat(Repeat::Infinite).map_err(err)?;
        let delay = image::Delay::from_numer_denom_ms(delay_ms, 1);
        for f in frames {
            enc.encode_frame(Frame::from_parts(rgba(f), 0, 0, delay)).map_err(err)?;
        }
    }
    Ok(out)
}

/// Animiertes PNG (APNG, Endlosschleife), jedes Bild `delay_ms` lang; alle Bilder gleich groß.
pub fn apng(frames: &[Pixmap], delay_ms: u32) -> Result<Vec<u8>, String> {
    let (w, h) = check_frames(frames)?;
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        // 0 Wiederholungen = endlos
        enc.set_animated(frames.len() as u32, 0).map_err(err)?;
        enc.set_frame_delay(delay_ms.min(u16::MAX as u32) as u16, 1000).map_err(err)?;
        let mut wr = enc.write_header().map_err(err)?;
        for f in frames {
            wr.write_image_data(rgba(f).as_raw()).map_err(err)?;
        }
        wr.finish().map_err(err)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::AnimationDecoder;

    fn bild(c: tiny_skia::Color) -> Pixmap {
        let mut pm = Pixmap::new(37, 21).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 0));
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(c);
        let r = tiny_skia::Rect::from_xywh(5.0, 4.0, 20.0, 10.0).unwrap();
        pm.fill_rect(r, &paint, tiny_skia::Transform::identity(), None);
        pm
    }

    fn rot() -> Pixmap {
        bild(tiny_skia::Color::from_rgba8(220, 30, 30, 255))
    }

    #[test]
    fn einzelformate() {
        let pm = rot();
        let bg = 0xFFFFFFFF;
        let faelle: [(Raster, &[u8]); 5] = [
            (Raster::Jpeg { quality: 80 }, &[0xFF, 0xD8, 0xFF]),
            (Raster::Webp, b"RIFF"),
            (Raster::Tiff, b"II*\0"),
            (Raster::Bmp, b"BM"),
            (Raster::Gif, b"GIF89a"),
        ];
        for (k, magic) in faelle {
            let b = encode(&pm, k, bg).unwrap_or_else(|e| panic!("{k:?}: {e}"));
            assert!(b.starts_with(magic), "{k:?}: Magic");
            let img = image::load_from_memory(&b).unwrap_or_else(|e| panic!("{k:?}: {e}"));
            assert_eq!((img.width(), img.height()), (37, 21), "{k:?}");
        }
    }

    #[test]
    fn jpeg_legt_auf_hintergrund() {
        let b = encode(&rot(), Raster::Jpeg { quality: 90 }, 0xFF00FF00).unwrap();
        let img = image::load_from_memory(&b).unwrap().to_rgb8();
        let p = img.get_pixel(33, 18);
        assert!(p[1] > 200 && p[0] < 60, "Ecke grün: {p:?}");
    }

    #[test]
    fn webp_behaelt_alpha() {
        let b = encode(&rot(), Raster::Webp, 0xFFFFFFFF).unwrap();
        let img = image::load_from_memory(&b).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0)[3], 0);
        assert_eq!(img.get_pixel(10, 8).0, [220, 30, 30, 255]);
    }

    #[test]
    fn avif_kodiert() {
        let b = encode(&rot(), Raster::Avif, 0xFFFFFFFF).unwrap();
        assert_eq!(&b[4..8], b"ftyp");
        assert!(b[8..32].windows(4).any(|w| w == b"avif"));
    }

    #[test]
    fn gif_animation() {
        let frames = [rot(), bild(tiny_skia::Color::from_rgba8(20, 60, 220, 255)), bild(tiny_skia::Color::from_rgba8(20, 200, 60, 255))];
        let b = gif_anim(&frames, 120).unwrap();
        assert!(b.starts_with(b"GIF89a"));
        assert!(b.windows(11).any(|w| w == b"NETSCAPE2.0"), "Schleife");
        let dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(b)).unwrap();
        let fr = dec.into_frames().collect_frames().unwrap();
        assert_eq!(fr.len(), 3);
        assert_eq!(fr[0].buffer().dimensions(), (37, 21));
        assert_eq!(fr[0].delay().numer_denom_ms().0 / fr[0].delay().numer_denom_ms().1, 120);
    }

    #[test]
    fn apng_animation() {
        let frames = [rot(), bild(tiny_skia::Color::from_rgba8(20, 60, 220, 255))];
        let b = apng(&frames, 120).unwrap();
        assert!(b.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(b.windows(4).any(|w| w == b"acTL"));
        assert_eq!(b.windows(4).filter(|w| *w == b"fcTL").count(), 2);
        let img = image::load_from_memory(&b).unwrap();
        assert_eq!((img.width(), img.height()), (37, 21));
    }

    #[test]
    fn leere_oder_ungleiche_bilder() {
        assert!(gif_anim(&[], 100).is_err());
        assert!(apng(&[rot(), Pixmap::new(3, 3).unwrap()], 100).is_err());
    }
}
