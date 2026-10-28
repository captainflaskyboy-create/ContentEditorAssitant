use crate::{
    error::{Error, Result},
    graphics::{self, Compositor, Graphics},
    media::{self, VideoReader},
    model::Acceleration,
    render,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};
use windows::Win32::Graphics::Direct2D::{
    Common::D2D_RECT_F, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_CPU_READ,
    D2D1_BITMAP_OPTIONS_TARGET,
};

pub fn review(
    source: PathBuf,
    output: PathBuf,
    start: f64,
    duration: Option<f64>,
    interval: f64,
    cancel: &Arc<AtomicBool>,
) -> Result<Value> {
    let m = media::inspect(&source)?;
    if !start.is_finite()
        || start < 0.0
        || start >= m.duration
        || !interval.is_finite()
        || interval < 0.1
        || duration.is_some_and(|v| !v.is_finite() || v <= 0.0)
    {
        return Err(Error::new(
            "invalid_arguments",
            "Review requires a valid source start, positive duration, and interval >=0.1 seconds",
        ));
    }
    let end = duration.map_or(m.duration, |d| (start + d).min(m.duration));
    if output.exists() {
        return Err(Error::new(
            "output_exists",
            "Review output directory must be new",
        ));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let _lock = render::lock()?;
    let stage = tempfile::Builder::new()
        .prefix(".acea-review-")
        .tempdir_in(parent)?;
    let g = Graphics::new(Acceleration::Auto)?;
    let r = VideoReader::new(
        &m.source,
        Acceleration::Auto,
        if g.hardware { Some(&g.manager) } else { None },
    )?;
    let tw = 320u32;
    let th = (m.height as f64 / m.width as f64 * tw as f64).round() as u32;
    if th > 4096 {
        return Err(Error::new(
            "invalid_media",
            "Review thumbnails would be too tall; normalize extreme portrait footage first",
        ));
    }
    let rowheight = th + 28;
    let sw = tw * 4;
    let sh = rowheight * 4;
    let c = Compositor::new(&g, m.width, m.height, tw, th)?;
    let req = render::empty_request(&m);
    let mut entries = vec![];
    let mut sheets = vec![];
    let mut t = start;
    let mut count = 0;
    while t < end - 1e-7 && count < 240 {
        let sheet_index = sheets.len();
        let name = format!("sheet-{sheet_index:03}.png");
        let mut data = vec![0u8; (sw * sh * 4) as usize];
        for p in data.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
        let mut labels = vec![];
        for cell in 0..16 {
            if t >= end - 1e-7 || count >= 240 {
                break;
            }
            media::check_cancel(cancel)?;
            let frame = r.at(t, cancel)?;
            let (input, _) = g.input(&r, &frame, m.rotation, &c.input)?;
            c.draw(&input, &req, 0.0)?;
            let pixels = c.pixels()?;
            let x = (cell % 4) as u32 * tw;
            let y = (cell / 4) as u32 * rowheight;
            for yy in 0..th as usize {
                let to = (((y as usize + yy) * sw as usize + x as usize) * 4) as usize;
                data[to..to + tw as usize * 4]
                    .copy_from_slice(&pixels[yy * tw as usize * 4..(yy + 1) * tw as usize * 4]);
            }
            let actual = frame.time as f64 / media::TICKS;
            let label = format!(
                "{:02}:{:06.3}",
                (actual / 60.0).floor() as u64,
                actual % 60.0
            );
            labels.push((
                label,
                D2D_RECT_F {
                    left: x as f32 + 4.0,
                    top: (y + th) as f32,
                    right: (x + tw) as f32,
                    bottom: (y + rowheight) as f32,
                },
            ));
            entries.push(json!({"requested_source_time":t,"actual_source_time":actual,"sheet":name,"cell":cell,"image_rect":{"x":x,"y":y,"width":tw,"height":th},"source_width":m.width,"source_height":m.height}));
            count += 1;
            t = start + count as f64 * interval;
        }
        let bitmap = g.bitmap(sw, sh, D2D1_BITMAP_OPTIONS_TARGET)?;
        unsafe {
            bitmap.CopyFromMemory(None, data.as_ptr().cast(), sw * 4)?;
        }
        graphics::label_sheet(&g, &bitmap, &labels)?;
        let cpu = g.bitmap(
            sw,
            sh,
            D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        )?;
        graphics::save_png(
            &stage.path().join(&name),
            sw,
            sh,
            &g.readback(&bitmap, &cpu)?,
        )?;
        sheets.push(name);
        media::event(
            json!({"event":"review_progress","sampled_frames":count,"source_seconds":t.min(end)}),
        );
    }
    let next = if t < end - 1e-7 { Some(t) } else { None };
    let index = json!({"version":2,"source":m,"interval":interval,"start":start,"end":end,"samples":entries,"sheets":sheets,"continuation_timestamp":next,"coordinates":"Each cell shows the entire source. Normalize targets within its image_rect, excluding the timestamp label."});
    std::fs::write(
        stage.path().join("index.json"),
        serde_json::to_vec_pretty(&index)?,
    )?;
    media::check_cancel(cancel)?;
    std::fs::rename(stage.path(), &output)?;
    Ok(
        json!({"ok":true,"index":output.join("index.json").canonicalize()?,"output_dir":output.canonicalize()?,"sampled_frames":count,"sheets":sheets,"continuation_timestamp":next}),
    )
}
