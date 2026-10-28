//! FFmpeg is a development fixture/oracle only. Every ACEA subprocess has PATH="".
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
fn acea(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_acea"))
        .env("PATH", "")
        .args(args)
        .output()
        .unwrap()
}
fn ok(out: Output) -> Value {
    let v: Value =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("Non-JSON: {out:?}"));
    assert!(
        out.status.success(),
        "{v}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    v
}
fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}
fn ffmpeg(args: &[&str]) -> Output {
    let out = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-v",
            "error",
            "-nostdin",
            "-threads",
            "2",
            "-filter_threads",
            "2",
        ])
        .args(args)
        .output()
        .expect("FFmpeg development fixture tool must be on PATH");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}
fn fixture(dir: &Path, audio: bool) -> PathBuf {
    let p = dir.join(if audio {
        "recording ü audio.mp4"
    } else {
        "recording ü silent.mp4"
    });
    let mut args=vec!["-f","lavfi","-i","color=c=gray:s=320x180:r=30:d=3,drawbox=x=224:y=64:w=32:h=52:color=green:t=fill,drawgrid=w=8:h=8:t=1:color=white"];
    if audio {
        args.extend([
            "-itsoffset",
            "0.5",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2.5",
            "-c:a",
            "aac",
        ]);
    }
    args.extend([
        "-c:v",
        "libx264",
        "-crf",
        "12",
        "-pix_fmt",
        "yuv420p",
        "-t",
        "3",
        "-y",
        s(&p),
    ]);
    ffmpeg(&args);
    p
}
fn write(p: &Path, v: &Value) {
    fs::write(p, serde_json::to_vec(v).unwrap()).unwrap();
}
fn render(d: &Path, source: &Path, name: &str, effects: Value) -> (PathBuf, Value) {
    let out = d.join(format!("{name}.mp4"));
    let req = d.join(format!("{name}.json"));
    write(
        &req,
        &json!({"version":2,"source":source,"output":out,"range":{"start":0,"duration":2},"effects":effects}),
    );
    let v = ok(acea(&["render", "--request", s(&req)]));
    (out, v)
}
fn pixels(p: &Path, time: f64) -> Vec<u8> {
    ffmpeg(&[
        "-ss",
        &time.to_string(),
        "-i",
        s(p),
        "-frames:v",
        "1",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "-",
    ])
    .stdout
}
fn mean(data: &[u8], x: usize, y: usize, w: usize, h: usize, c: usize) -> f64 {
    let mut n = 0.0;
    for yy in y..y + h {
        for xx in x..x + w {
            n += data[(yy * 320 + xx) * 3 + c] as f64;
        }
    }
    n / (w * h) as f64
}
fn variance(data: &[u8], x: usize, y: usize, w: usize, h: usize) -> f64 {
    let m = mean(data, x, y, w, h, 0);
    let mut n = 0.0;
    for yy in y..y + h {
        for xx in x..x + w {
            n += (data[(yy * 320 + xx) * 3] as f64 - m).powi(2);
        }
    }
    n / (w * h) as f64
}

#[test]
fn native_doctor_and_capabilities_need_no_path_tools() {
    let v = ok(acea(&["doctor"]));
    assert_eq!(v["backend"], "windows_native");
    assert_eq!(v["external_dependencies"], json!([]));
    assert!(v["graphics"]["adapter"].is_string());
}
#[test]
fn rejects_version_one_before_opening_media() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("v1.json");
    write(
        &p,
        &json!({"version":1,"source":"missing","output":"out.mp4","range":{"start":0,"duration":1}}),
    );
    let out = acea(&["validate", "--request", s(&p)]);
    assert_eq!(out.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("version:2"));
}

#[test]
#[ignore = "requires FFmpeg as development oracle"]
fn rendered_geometry_spotlight_blur_and_overlap() {
    let d = tempfile::tempdir().unwrap();
    let source = fixture(d.path(), false);
    let (base, meta) = render(d.path(), &source, "base", json!([]));
    assert_eq!(meta["has_audio"], false);
    assert!((meta["duration"].as_f64().unwrap() - 2.0).abs() < 0.05);
    let (zoom, _) = render(
        d.path(),
        &source,
        "snap",
        json!([{"type":"snap_zoom","start":0.5,"end":1.5,"zoom":2,"target":{"x":0.75,"y":0.5}}]),
    );
    let before = pixels(&zoom, 0.2);
    let focused = pixels(&zoom, 0.9);
    let after = pixels(&zoom, 1.8);
    assert!(mean(&before, 150, 80, 20, 20, 0) > 100.0);
    assert!(mean(&focused, 150, 80, 20, 20, 1) - mean(&focused, 150, 80, 20, 20, 0) > 50.0);
    assert!(mean(&after, 150, 80, 20, 20, 0) > 100.0);
    let (spot, _) = render(
        d.path(),
        &source,
        "spot",
        json!([{"type":"spotlight","start":0,"end":2,"rect":{"x":0.65,"y":0.3,"width":0.2,"height":0.4},"dim":0.7}]),
    );
    let original = pixels(&base, 0.5);
    let lit = pixels(&spot, 0.5);
    assert!(mean(&lit, 30, 30, 40, 40, 0) < mean(&original, 30, 30, 40, 40, 0) * 0.5);
    assert!((mean(&lit, 230, 70, 20, 40, 1) - mean(&original, 230, 70, 20, 40, 1)).abs() < 10.0);
    let (blur, _) = render(
        d.path(),
        &source,
        "blur",
        json!([{"type":"blur","start":0,"end":2,"rect":{"x":0,"y":0,"width":0.5,"height":1},"sigma":5}]),
    );
    let blurred = pixels(&blur, 0.5);
    assert!(variance(&blurred, 20, 20, 80, 80) < variance(&original, 20, 20, 80, 80) * 0.25);
    assert!(
        (mean(&blurred, 230, 70, 20, 40, 1) - mean(&original, 230, 70, 20, 40, 1)).abs() < 10.0
    );
    render(
        d.path(),
        &source,
        "combined",
        json!([{"type":"zoom","start":0,"end":2,"target":{"x":1,"y":0}},{"type":"spotlight","start":0,"end":2,"rect":{"x":0.6,"y":0.2,"width":0.3,"height":0.5}},{"type":"blur","start":0,"end":2,"rect":{"x":0.7,"y":0.1,"width":0.2,"height":0.2}}]),
    );
}

#[test]
#[ignore = "requires FFmpeg as development oracle"]
fn easing_and_camera_paths() {
    let d = tempfile::tempdir().unwrap();
    let source = fixture(d.path(), false);
    for (e, expected) in [
        ("linear", 220.0),
        ("ease_in", 235.0),
        ("ease_out", 205.0),
        ("ease_in_out", 230.0),
    ] {
        let (p, _) = render(
            d.path(),
            &source,
            e,
            json!([{"type":"zoom","start":0,"end":2,"from_zoom":1,"to_zoom":2,"target":{"x":0.75,"y":0.5},"easing":e}]),
        );
        let b = pixels(&p, 0.5);
        let xs: Vec<f64> = (0..320)
            .filter(|x| {
                let i = (90 * 320 + x) * 3;
                b[i + 1] as i32 - b[i] as i32 > 40
            })
            .map(|x| x as f64)
            .collect();
        let center = xs.iter().sum::<f64>() / xs.len() as f64;
        assert!((center - expected).abs() < 4.0, "{e}: {center}");
    }
    render(
        d.path(),
        &source,
        "ken",
        json!([{"type":"ken_burns","start":0,"end":2,"from":{"zoom":1.2,"target":{"x":0.3,"y":0.4}},"to":{"zoom":1.8,"target":{"x":0.7,"y":0.6}}}]),
    );
    render(
        d.path(),
        &source,
        "path",
        json!([{"type":"camera","keyframes":[{"time":0,"zoom":1},{"time":0.5,"zoom":1.8},{"time":1.4,"zoom":1.8},{"time":2,"zoom":1}]}]),
    );
}

#[test]
#[ignore = "requires FFmpeg as development oracle"]
fn delayed_audio_remains_synchronized() {
    let d = tempfile::tempdir().unwrap();
    let source = fixture(d.path(), true);
    let req = d.path().join("audio.json");
    let out = d.path().join("audio.mp4");
    write(
        &req,
        &json!({"version":2,"source":source,"output":out,"range":{"start":0.1,"duration":1.6}}),
    );
    let v = ok(acea(&["render", "--request", s(&req)]));
    assert_eq!(v["has_audio"], true);
    let pcm = ffmpeg(&[
        "-i",
        s(&out),
        "-vn",
        "-f",
        "s16le",
        "-ac",
        "1",
        "-ar",
        "8000",
        "-",
    ])
    .stdout;
    let samples: Vec<f64> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|x| i16::from_le_bytes(*x) as f64)
        .collect();
    let energy =
        |a: usize, b: usize| samples[a..b].iter().map(|v| v.abs()).sum::<f64>() / (b - a) as f64;
    assert!(energy(0, 1600) < 25.0);
    assert!(energy(6400, 8000) > 500.0);
}

#[test]
#[ignore = "requires FFmpeg as development oracle"]
fn sampling_review_preview_software_and_protected_overwrite() {
    let d = tempfile::tempdir().unwrap();
    let source = fixture(d.path(), false);
    let frames = d.path().join("frames ü");
    let v = ok(acea(&[
        "frames",
        "--source",
        s(&source),
        "--output-dir",
        s(&frames),
        "--times",
        "0,1,2",
        "--height",
        "144",
    ]));
    assert_eq!(v["frames"].as_array().unwrap().len(), 3);
    assert!(v["frames"][1]["actual_source_time"].as_f64().unwrap() >= 1.0 - 1e-6);
    let review = d.path().join("review");
    let v = ok(acea(&[
        "review",
        "--source",
        s(&source),
        "--output-dir",
        s(&review),
        "--interval",
        "1",
    ]));
    assert_eq!(v["sampled_frames"], 3);
    assert!(review.join("sheet-000.png").is_file());
    assert!(review.join("index.json").is_file());
    let req = d.path().join("preview.json");
    let out = d.path().join("preview.mp4");
    let mut body = json!({"version":2,"source":source,"output":out,"range":{"start":0,"duration":1},"settings":{"preview_height":144,"acceleration":"software"},"effects":[{"type":"pan","start":0,"end":1,"from":{"x":0,"y":0},"to":{"x":1,"y":1}}]});
    write(&req, &body);
    ok(acea(&["validate", "--request", s(&req)]));
    assert!(!out.exists());
    let v = ok(acea(&["render", "--request", s(&req)]));
    assert_eq!(v["height"], 144);
    assert_eq!(v["acceleration"]["graphics"]["hardware"], false);
    assert_eq!(v["acceleration"]["encoder"]["hardware_observed"], false);
    assert_eq!(
        acea(&["render", "--request", s(&req)]).status.code(),
        Some(2)
    );
    body["overwrite"] = json!(true);
    write(&req, &body);
    ok(acea(&["render", "--request", s(&req)]));
    let original = fs::read(&out).unwrap();
    body["effects"][0]["end"] = json!(9);
    write(&req, &body);
    assert!(!acea(&["render", "--request", s(&req)]).status.success());
    assert_eq!(fs::read(&out).unwrap(), original);
}

#[test]
#[ignore = "requires FFmpeg as development oracle"]
fn batch_order_resume_corruption_edit_and_source_changes() {
    let d = tempfile::tempdir().unwrap();
    let source = fixture(d.path(), false);
    let output = d.path().join("job");
    let req = d.path().join("job.json");
    let mut job = json!({"version":2,"source":source,"output_dir":output,"max_duration":3,"clips":[{"id":"a","label":"Opening","range":{"start":0,"duration":1}},{"id":"b","label":"Payoff","range":{"start":1,"duration":1}}]});
    write(&req, &job);
    let valid = ok(acea(&["validate", "--request", s(&req)]));
    assert_eq!(valid["clips"].as_array().unwrap().len(), 2);
    assert!(!output.exists());
    let v = ok(acea(&["batch", "--request", s(&req)]));
    assert_eq!(v["rendered"], 2);
    job["clips"].as_array_mut().unwrap().reverse();
    write(&req, &job);
    let v = ok(acea(&["batch", "--request", s(&req), "--resume", "true"]));
    assert_eq!(v["reused"], 2);
    assert_eq!(v["clips"][0]["id"], "b");
    fs::write(output.join("a.mp4"), b"corrupt").unwrap();
    let v = ok(acea(&["batch", "--request", s(&req), "--resume", "true"]));
    assert_eq!(v["rendered"], 1);
    assert_eq!(v["reused"], 1);
    job["clips"][0]["effects"] = json!([{"type":"snap_zoom","start":0,"end":1,"zoom":1.2}]);
    write(&req, &job);
    let v = ok(acea(&["batch", "--request", s(&req), "--resume", "true"]));
    assert_eq!(v["rendered"], 1);
    assert_eq!(v["reused"], 1);
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(&source)
        .unwrap()
        .write_all(b"changed-source-identity")
        .unwrap();
    let v = ok(acea(&["batch", "--request", s(&req), "--resume", "true"]));
    assert_eq!(v["rendered"], 2);
    job["max_duration"] = json!(0.5);
    write(&req, &job);
    assert_eq!(
        acea(&["validate", "--request", s(&req)]).status.code(),
        Some(2)
    );
    job["max_duration"] = json!(3);
    job["clips"][1]["id"] = json!("B");
    write(&req, &job);
    assert_eq!(
        acea(&["validate", "--request", s(&req)]).status.code(),
        Some(2)
    );
}

#[test]
#[ignore = "requires FFmpeg as development oracle"]
fn rotation_and_sixty_fps() {
    let d = tempfile::tempdir().unwrap();
    let source = fixture(d.path(), false);
    let rotated = d.path().join("rotated.mp4");
    ffmpeg(&[
        "-display_rotation",
        "90",
        "-i",
        s(&source),
        "-c",
        "copy",
        "-y",
        s(&rotated),
    ]);
    let info = ok(acea(&["inspect", "--source", s(&rotated)]));
    assert_eq!(info["media"]["width"], 180);
    let (_, v) = render(d.path(), &rotated, "rotation", json!([]));
    assert_eq!(v["width"], 180);
    assert_eq!(v["height"], 320);
    let sixty = d.path().join("sixty.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=320x180:r=60:d=2",
        "-c:v",
        "libx264",
        "-threads",
        "2",
        "-y",
        s(&sixty),
    ]);
    let (_, v) = render(d.path(), &sixty, "fps60", json!([]));
    assert_eq!(v["frame_rate"], "60/1");
}

#[test]
#[ignore = "requires FFmpeg as development oracle"]
fn lock_rejects_parallel_jobs() {
    use fs2::FileExt;
    let d = tempfile::tempdir().unwrap();
    let source = fixture(d.path(), false);
    let f = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(std::env::temp_dir().join("ai-content-editor-assistant-render-v1.lock"))
        .unwrap();
    f.lock_exclusive().unwrap();
    let req = d.path().join("busy.json");
    write(
        &req,
        &json!({"version":2,"source":source,"output":d.path().join("out.mp4"),"range":{"start":0,"duration":1}}),
    );
    assert_eq!(
        acea(&["render", "--request", s(&req)]).status.code(),
        Some(3)
    );
}

#[test]
#[ignore = "requires FFmpeg as development oracle"]
fn quantized_mkv_audio_timestamps_do_not_create_crackle() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("quantized-audio.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=gray:s=320x180:r=30:d=4",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=997:sample_rate=48000:duration=4",
        "-c:v",
        "libx264",
        "-threads",
        "2",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-y",
        s(&source),
    ]);
    let request = d.path().join("clip.json");
    let output = d.path().join("clip.mp4");
    write(
        &request,
        &json!({"version":2,"source":source,"output":output,"range":{"start":0.137,"duration":3},"settings":{"acceleration":"software"}}),
    );
    ok(acea(&["render", "--request", s(&request)]));
    let discontinuity = |p: &Path| {
        let pcm = ffmpeg(&[
            "-i",
            s(p),
            "-vn",
            "-ac",
            "1",
            "-ar",
            "48000",
            "-f",
            "f32le",
            "-",
        ])
        .stdout;
        let samples: Vec<f32> = pcm
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect();
        let samples = &samples[12000..samples.len() - 12000];
        let peak = samples.iter().fold(0.0f32, |p, v| p.max(v.abs()));
        samples
            .windows(2)
            .map(|p| (p[1] - p[0]).abs() / peak)
            .fold(0.0f32, f32::max)
    };
    let original = discontinuity(&source);
    let rendered = discontinuity(&output);
    assert!(
        original < 0.3,
        "Source fixture contains unexpected discontinuities: {original}"
    );
    assert!(rendered<0.3,"Rendering introduced impulsive audio discontinuities: source={original}, rendered={rendered}");
}
