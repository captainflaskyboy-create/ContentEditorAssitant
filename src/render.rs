use crate::{
    encoder::{Audio, Encoder, SurfacePool},
    error::{Error, Result},
    graphics::{self, Compositor, Graphics},
    media::{self, Media, VideoReader},
    model::*,
    validate,
};
use fs2::FileExt;
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

pub fn lock() -> Result<File> {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(std::env::temp_dir().join("ai-content-editor-assistant-render-v1.lock"))?;
    f.try_lock_exclusive().map_err(|_| {
        Error::new(
            "busy",
            "Another ACEA job is running; retry when it finishes",
        )
    })?;
    Ok(f)
}
pub fn load_json(path: &str) -> Result<(Value, PathBuf)> {
    let mut raw = vec![];
    let base = if path == "-" {
        std::io::stdin().take(4_194_305).read_to_end(&mut raw)?;
        std::env::current_dir()?
    } else {
        let p = Path::new(path).canonicalize()?;
        File::open(&p)?.take(4_194_305).read_to_end(&mut raw)?;
        p.parent().unwrap().to_path_buf()
    };
    if raw.len() > 4_194_304 {
        return Err(Error::new("invalid_request", "Request exceeds 4 MiB"));
    }
    let v: Value = serde_json::from_slice(raw.strip_prefix(&[239, 187, 191]).unwrap_or(&raw))?;
    if v["version"] != 2 {
        return Err(Error::new("invalid_request","Native ACEA requires explicit version:2. Replace settings.crf with video_bitrate_kbps; run acea capabilities for the native schema."));
    }
    Ok((v, base))
}
pub fn load(path: &str) -> Result<Request> {
    let (v, base) = load_json(path)?;
    let mut r: Request = serde_json::from_value(v)?;
    if r.source.is_relative() {
        r.source = base.join(&r.source);
    }
    if r.output.is_relative() {
        r.output = base.join(&r.output);
    }
    Ok(r)
}
pub fn validate_request(req: &Request) -> Result<(Media, Vec<String>)> {
    let m = media::inspect(&req.source)?;
    let warnings = validate::validate(req, &m)?;
    // Negotiation detects unsupported installed codecs before a job writes any clips.
    let decoder = VideoReader::new(&m.source, req.settings.acceleration, None)?;
    let _ = decoder.at(req.range.start, &Arc::new(AtomicBool::new(false)))?;
    if m.has_audio {
        let _ = Audio::new(&m.source, req.range.start)?;
    }
    Ok((m, warnings))
}
pub fn render(req: Request, cancel: &Arc<AtomicBool>) -> Result<Value> {
    let _lock = lock()?;
    render_unlocked(req, cancel)
}
pub fn render_unlocked(req: Request, cancel: &Arc<AtomicBool>) -> Result<Value> {
    match render_attempt(req.clone(), cancel, true) {
        Err(e) if e.code == "gpu_surface_failed" => {
            media::event(
                json!({"event":"native_fallback","reason":e.message,"path":"system_memory_encoder_input"}),
            );
            let mut v = render_attempt(req, cancel, false)?;
            v["warnings"].as_array_mut().unwrap().push(json!(e.message));
            Ok(v)
        }
        Err(e)
            if e.message.contains("0x887A000")
                && req.settings.acceleration == Acceleration::Auto =>
        {
            let mut r = req;
            r.settings.acceleration = Acceleration::Software;
            media::event(json!({"event":"native_fallback","reason":e.message,"path":"software"}));
            let mut v = render_attempt(r, cancel, false)?;
            v["warnings"].as_array_mut().unwrap().push(json!(e.message));
            Ok(v)
        }
        other => other,
    }
}
fn render_attempt(req: Request, cancel: &Arc<AtomicBool>, allow_gpu: bool) -> Result<Value> {
    let started = Instant::now();
    media::check_cancel(cancel)?;
    let (source, mut warnings) = validate_request(&req)?;
    let g = Graphics::new(req.settings.acceleration)?;
    let reader = VideoReader::new(
        &source.source,
        req.settings.acceleration,
        if g.hardware { Some(&g.manager) } else { None },
    )?;
    warnings.extend(reader.warnings.iter().cloned());
    let (w, h) = graphics::dimensions(&req, &source);
    let compositor = Compositor::new(&g, source.width, source.height, w, h)?;
    let parent = req.output.parent().unwrap_or(Path::new("."));
    let temp = tempfile::Builder::new()
        .prefix(".acea-")
        .suffix(".partial.mp4")
        .tempfile_in(parent)?
        .into_temp_path();
    let encoder = Encoder::new(&temp, &req, &source, &g, allow_gpu)?;
    let encoding = encoder.info.clone();
    let surfaces = if encoder.gpu_input {
        Some(SurfacePool::new(&g, w, h)?)
    } else {
        None
    };
    let frames = (req.range.duration * source.fps + 1e-6).floor() as u64;
    let duration = frames as f64 / source.fps;
    let mut audio = if source.has_audio {
        Some(Audio::new(&source.source, req.range.start)?)
    } else {
        None
    };
    let mut frame = reader.at(req.range.start, cancel)?;
    let mut next = reader.next()?;
    let mut direct_frames = 0;
    let mut last_progress = Instant::now();
    for i in 0..frames {
        media::check_cancel(cancel)?;
        let t = i as f64 / source.fps;
        let source_time = media::ticks(req.range.start + t);
        while next.as_ref().is_some_and(|f| f.time <= source_time) {
            frame = next.take().unwrap();
            next = reader.next()?;
        }
        let (input, direct) = g.input(&reader, &frame, source.rotation, &compositor.input)?;
        if direct {
            direct_frames += 1;
        }
        compositor.draw(&input, &req, t)?;
        let a = media::ticks(t);
        let b = media::ticks((i + 1) as f64 / source.fps);
        if let Some(pool) = &surfaces {
            pool.write(&g, &encoder, &compositor.output, a, b - a, cancel)?;
        } else {
            let output = compositor.pixels()?;
            encoder.video(&output, a, b - a)?;
        }
        if let Some(audio) = &mut audio {
            audio.write_until(
                &encoder,
                (((i + 1) as f64 / source.fps).min(duration) * 48000.0).round() as i64,
                cancel,
            )?;
        }
        if last_progress.elapsed().as_secs_f64() >= 0.5 || i + 1 == frames {
            media::event(
                json!({"event":"render_progress","completed_frames":i+1,"total_frames":frames,"media_seconds":(i+1) as f64/source.fps,"elapsed_seconds":started.elapsed().as_secs_f64()}),
            );
            last_progress = Instant::now();
        }
    }
    encoder.finish()?;
    media::check_cancel(cancel)?;
    let result = media::inspect(&temp)?;
    if result.width != w
        || result.height != h
        || result.has_audio != source.has_audio
        || (result.duration - duration).abs() > 2.0 / source.fps + 0.05
    {
        return Err(Error::new("output_verification_failed","Native output dimensions, duration, or audio do not match the request; partial output discarded"));
    }
    let seconds = started.elapsed().as_secs_f64();
    if req.overwrite {
        temp.persist(&req.output)
            .map_err(|e| Error::new("publish_failed", e.to_string()))?;
    } else {
        temp.persist_noclobber(&req.output)
            .map_err(|e| Error::new("publish_failed", e.to_string()))?;
    }
    warnings.push("Native codec components manage their own worker threads; settings.threads is an application work limit, not a system-wide thread cap".into());
    Ok(
        json!({"ok":true,"output":req.output.canonicalize()?,"duration":result.duration,"width":w,"height":h,"frame_rate":source.frame_rate,"has_audio":source.has_audio,"video_bitrate_kbps":graphics::bitrate(&req,&source),"processing_seconds":seconds,"render_speed":duration/seconds,"warnings":warnings,"acceleration":{"graphics":g.info,"decoder_transforms":reader.transforms,"direct_decoder_texture_frames":direct_frames,"total_frames":frames,"encoder":encoding,"output_transfer":if surfaces.is_some(){"bounded D3D11 texture pool; no CPU frame readback"}else{"bounded BGRA readback; native encoder conversion"}}}),
    )
}

pub fn frames(
    source: PathBuf,
    directory: PathBuf,
    times: Vec<f64>,
    height: u32,
    cancel: &Arc<AtomicBool>,
) -> Result<Value> {
    if times.is_empty() || times.len() > 12 {
        return Err(Error::new(
            "invalid_arguments",
            "Supply 1..12 source timestamps",
        ));
    }
    if !(2..=1080).contains(&height) {
        return Err(Error::new(
            "invalid_arguments",
            "Frame height must be 2..1080",
        ));
    }
    let m = media::inspect(&source)?;
    if times
        .iter()
        .any(|t| !t.is_finite() || *t < 0.0 || *t >= m.duration)
    {
        return Err(Error::new(
            "invalid_arguments",
            "Timestamps must lie within the source",
        ));
    }
    if directory.exists() {
        return Err(Error::new(
            "output_exists",
            "Frame directory must not already exist",
        ));
    }
    let parent = directory
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let _lock = lock()?;
    let stage = tempfile::Builder::new()
        .prefix(".acea-frames-")
        .tempdir_in(parent)?;
    let g = Graphics::new(Acceleration::Auto)?;
    let r = VideoReader::new(
        &m.source,
        Acceleration::Auto,
        if g.hardware { Some(&g.manager) } else { None },
    )?;
    let h = height.min(m.height);
    let w = (m.width as f64 * h as f64 / m.height as f64).round() as u32;
    let c = Compositor::new(&g, m.width, m.height, w, h)?;
    let request = empty_request(&m);
    let mut entries = vec![];
    for (i, t) in times.iter().enumerate() {
        let f = r.at(*t, cancel)?;
        let (input, _) = g.input(&r, &f, m.rotation, &c.input)?;
        c.draw(&input, &request, 0.0)?;
        let name = format!("frame-{i:02}.png");
        graphics::save_png(&stage.path().join(&name), w, h, &c.pixels()?)?;
        entries.push(json!({"requested_source_time":t,"actual_source_time":f.time as f64/media::TICKS,"path":name}));
    }
    media::check_cancel(cancel)?;
    std::fs::rename(stage.path(), &directory)?;
    for e in &mut entries {
        e["path"] = json!(directory.join(e["path"].as_str().unwrap()).canonicalize()?);
    }
    Ok(
        json!({"ok":true,"source":m,"frames":entries,"coordinates":"Normalized full displayed source frame"}),
    )
}
pub fn empty_request(m: &Media) -> Request {
    Request {
        version: 2,
        source: m.source.clone(),
        output: "unused.mp4".into(),
        range: Range {
            start: 0.0,
            duration: m.duration,
        },
        effects: vec![],
        settings: Settings::default(),
        overwrite: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "uses supplied recording and Windows Media Foundation"]
    fn cancelling_a_running_render_preserves_final_and_cleans_partial() {
        let _runtime = media::Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("video-clips-for-testing/pngtuber_speedrun.mkv");
        let output = dir.path().join("final.mp4");
        std::fs::write(&output, b"previous-final").unwrap();
        let req:Request=serde_json::from_value(json!({"version":2,"source":source,"output":output,"range":{"start":10,"duration":20},"settings":{"acceleration":"software"},"overwrite":true})).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let trigger = cancel.clone();
        let setter = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(1));
            trigger.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let result = render(req, &cancel);
        setter.join().unwrap();
        assert_eq!(result.unwrap_err().code, "cancelled");
        assert_eq!(std::fs::read(&output).unwrap(), b"previous-final");
        assert!(!std::fs::read_dir(dir.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with("partial.mp4")));
    }
}
