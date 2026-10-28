use crate::{
    error::{Error, Result},
    graphics,
    media::{self, Media},
    model::*,
    render, validate,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub version: u32,
    pub source: PathBuf,
    pub output_dir: PathBuf,
    #[serde(default)]
    pub max_duration: Option<f64>,
    #[serde(default)]
    pub settings: Settings,
    pub clips: Vec<Clip>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub range: Range,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub settings: Option<Settings>,
}
pub fn load(path: &str) -> Result<Job> {
    let (v, base) = render::load_json(path)?;
    from_value(v, &base)
}
pub fn from_value(v: Value, base: &Path) -> Result<Job> {
    let mut j: Job = serde_json::from_value(v)?;
    if j.source.is_relative() {
        j.source = base.join(&j.source);
    }
    if j.output_dir.is_relative() {
        j.output_dir = base.join(&j.output_dir);
    }
    Ok(j)
}
pub fn clip_request(job: &Job, c: &Clip, overwrite: bool) -> Request {
    Request {
        version: job.version,
        source: job.source.clone(),
        output: job.output_dir.join(format!("{}.mp4", c.id)),
        range: c.range.clone(),
        effects: c.effects.clone(),
        settings: c.settings.clone().unwrap_or_else(|| job.settings.clone()),
        overwrite,
    }
}
pub fn validate_job(job: &Job) -> Result<(Media, Vec<Value>)> {
    if job.version != 2 || job.clips.is_empty() || job.clips.len() > 1000 {
        return Err(Error::new(
            "invalid_request",
            "A version 2 job requires 1..1000 clips",
        ));
    }
    if job.max_duration.is_some_and(|v| !v.is_finite() || v <= 0.0) {
        return Err(Error::new(
            "invalid_request",
            "max_duration must be positive seconds",
        ));
    }
    if job.output_dir.exists() && !job.output_dir.is_dir() {
        return Err(Error::new(
            "invalid_request",
            "output_dir must be a directory",
        ));
    }
    let parent = job.output_dir.parent().unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err(Error::new(
            "invalid_request",
            "Parent of output_dir must already exist",
        ));
    }
    let m = media::inspect(&job.source)?;
    let mut ids = HashSet::new();
    let mut clips = vec![];
    let mut total = 0.0;
    for c in &job.clips {
        let reserved = c.id.eq_ignore_ascii_case("CON")
            || c.id.eq_ignore_ascii_case("PRN")
            || c.id.eq_ignore_ascii_case("AUX")
            || c.id.eq_ignore_ascii_case("NUL")
            || (1..=9).any(|n| {
                c.id.eq_ignore_ascii_case(&format!("COM{n}"))
                    || c.id.eq_ignore_ascii_case(&format!("LPT{n}"))
            });
        if reserved
            || c.id.is_empty()
            || c.id.len() > 64
            || !c
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !ids.insert(c.id.to_ascii_lowercase())
        {
            return Err(Error::new("invalid_request","Clip IDs must be unique (case-insensitive), 1..64 ASCII letters, digits, '-' or '_'"));
        }
        let r = clip_request(job, c, false);
        let warnings = validate::validate_with_output(&r, &m, false)?;
        if r.output.canonicalize().ok().as_ref() == Some(&m.source) {
            return Err(Error::new(
                "invalid_request",
                "A clip output would replace the source",
            ));
        }
        let duration = (c.range.duration * m.fps + 1e-6).floor() / m.fps
            + if m.has_audio { 1024.0 / 48000.0 } else { 0.0 };
        let (w, h) = graphics::dimensions(&r, &m);
        clips.push(json!({"id":c.id,"label":c.label,"source_range":c.range,"duration":duration,"timeline_start":total,"width":w,"height":h,"warnings":warnings}));
        total += duration;
    }
    if job.max_duration.is_some_and(|limit| total > limit + 1e-7) {
        return Err(Error::new(
            "duration_limit",
            format!("Selected clips total {total:.3}s, exceeding max_duration"),
        ));
    }
    // Check source decoder/audio negotiation before creating the job directory.
    let first = clip_request(job, &job.clips[0], false);
    let decoder = media::VideoReader::new(&m.source, first.settings.acceleration, None)?;
    let _ = decoder.at(first.range.start, &Arc::new(AtomicBool::new(false)))?;
    if m.has_audio {
        let _ = crate::encoder::Audio::new(&m.source, 0.0)?;
    }
    Ok((m, clips))
}
pub fn hash_file(path: &Path, cancel: &Arc<AtomicBool>) -> Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        media::check_cancel(cancel)?;
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub fn fingerprint(source_hash: &str, r: &Request) -> Result<String> {
    let mut h = Sha256::new();
    h.update(env!("CARGO_PKG_VERSION"));
    h.update(source_hash);
    h.update(serde_json::to_vec(
        &json!({"range":r.range,"effects":r.effects,"settings":r.settings}),
    )?);
    Ok(format!("{:x}", h.finalize()))
}
pub fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let mut tmp = tempfile::Builder::new()
        .prefix(".acea-manifest-")
        .tempfile_in(path.parent().unwrap_or(Path::new(".")))?;
    serde_json::to_writer_pretty(&mut tmp, value)?;
    tmp.write_all(b"\n")?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .map_err(|e| Error::new("publish_failed", e.to_string()))?;
    Ok(())
}
fn checkpoint_error(manifest: &mut Value, path: &Path, index: usize, error: &Error) -> Result<()> {
    manifest["status"] = json!(if error.code == "cancelled" {
        "cancelled"
    } else {
        "failed"
    });
    manifest["clips"][index]["status"] = manifest["status"].clone();
    manifest["clips"][index]["error"] = error.json()["error"].clone();
    atomic_json(path, manifest)
}

pub fn run(job: Job, resume: bool, cancel: &Arc<AtomicBool>) -> Result<Value> {
    let _lock = render::lock()?;
    let started = Instant::now();
    let (m, planned) = validate_job(&job)?;
    let manifest_path = job.output_dir.join("manifest.json");
    if job.output_dir.exists() && !resume {
        return Err(Error::new("output_exists","Job output directory already exists; use batch --resume true to validate and resume it"));
    }
    let old: Value = if manifest_path.is_file() {
        serde_json::from_reader(File::open(&manifest_path)?).map_err(|e| {
            Error::new(
                "invalid_manifest",
                format!("Cannot safely resume invalid manifest: {e}"),
            )
        })?
    } else {
        json!({})
    };
    if job.output_dir.exists() && old["version"] != 2 {
        return Err(Error::new(
            "invalid_manifest",
            "Existing job directory has no version 2 manifest; choose a fresh output directory",
        ));
    }
    let source_hash = hash_file(&m.source, cancel)?;
    let old_entries: HashMap<String, Value> = old["clips"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some((
                c["id"].as_str()?.to_string(),
                if c["status"] != "complete" && c["resume_candidate"].is_object() {
                    c["resume_candidate"].clone()
                } else {
                    c.clone()
                },
            ))
        })
        .collect();
    for c in &job.clips {
        if job.output_dir.join(format!("{}.mp4", c.id)).exists() && !old_entries.contains_key(&c.id)
        {
            return Err(Error::new(
                "output_exists",
                format!(
                    "Unmanaged output {}.mp4 already exists; choose another clip ID",
                    c.id
                ),
            ));
        }
    }
    let mut entries = planned;
    for e in &mut entries {
        e["status"] = json!("pending");
        if let Some(previous) = old_entries.get(e["id"].as_str().unwrap()) {
            e["resume_candidate"] = previous.clone();
        }
    }
    if !job.output_dir.exists() {
        std::fs::create_dir(&job.output_dir)?;
    }
    let mut manifest = json!({"version":2,"renderer_version":env!("CARGO_PKG_VERSION"),"source":m.source,"source_sha256":source_hash,"max_duration":job.max_duration,"status":"running","clips":entries});
    atomic_json(&manifest_path, &manifest)?;
    let mut reused = 0;
    let mut rendered = 0;
    for (i, c) in job.clips.iter().enumerate() {
        let r = clip_request(&job, c, true);
        let hash = fingerprint(&source_hash, &r)?;
        let old = old_entries.get(&c.id);
        let reusable = if let Some(previous) = old {
            if previous["status"] == "complete"
                && previous["fingerprint"] == hash
                && r.output.is_file()
            {
                let output_hash = match hash_file(&r.output, cancel) {
                    Ok(h) => h,
                    Err(e) => {
                        checkpoint_error(&mut manifest, &manifest_path, i, &e)?;
                        return Err(e);
                    }
                };
                previous["output_sha256"] == output_hash
                    && media::inspect(&r.output).is_ok_and(|out| {
                        let (w, h) = graphics::dimensions(&r, &m);
                        out.width == w
                            && out.height == h
                            && out.has_audio == m.has_audio
                            && (out.duration - manifest["clips"][i]["duration"].as_f64().unwrap())
                                .abs()
                                < 2.0 / m.fps + 0.05
                    })
            } else {
                false
            }
        } else {
            false
        };
        let attempt = if reusable {
            reused += 1;
            media::event(json!({"event":"clip_reused","id":c.id,"index":i}));
            Ok(old.unwrap()["result"].clone())
        } else {
            media::event(
                json!({"event":"clip_started","id":c.id,"index":i,"total_clips":job.clips.len()}),
            );
            let result = render::render_unlocked(r.clone(), cancel);
            if result.is_ok() {
                rendered += 1;
            }
            result
        };
        match attempt {
            Ok(result) => {
                let hash_out = match hash_file(&r.output, cancel) {
                    Ok(h) => h,
                    Err(e) => {
                        checkpoint_error(&mut manifest, &manifest_path, i, &e)?;
                        return Err(e);
                    }
                };
                manifest["clips"][i]
                    .as_object_mut()
                    .unwrap()
                    .remove("resume_candidate");
                manifest["clips"][i]["status"] = json!("complete");
                manifest["clips"][i]["fingerprint"] = json!(hash);
                manifest["clips"][i]["output_sha256"] = json!(hash_out);
                manifest["clips"][i]["output"] = json!(r.output.canonicalize()?);
                manifest["clips"][i]["result"] = result;
                manifest["clips"][i]["reused"] = json!(reusable);
                atomic_json(&manifest_path, &manifest)?;
            }
            Err(e) => {
                manifest["status"] = json!(if e.code == "cancelled" {
                    "cancelled"
                } else {
                    "failed"
                });
                manifest["clips"][i]["status"] = manifest["status"].clone();
                manifest["clips"][i]["error"] = e.json()["error"].clone();
                atomic_json(&manifest_path, &manifest)?;
                return Err(Error::new(
                    e.code,
                    format!(
                        "Clip {} failed: {}. Resume using manifest {}",
                        c.id,
                        e.message,
                        manifest_path.display()
                    ),
                ));
            }
        }
    }
    let mut total = 0.0;
    for e in manifest["clips"].as_array_mut().unwrap() {
        e["timeline_start"] = json!(total);
        let duration = e["result"]["duration"].as_f64().unwrap();
        e["duration"] = json!(duration);
        total += duration;
    }
    if job.max_duration.is_some_and(|limit| total > limit + 1e-6) {
        manifest["status"] = json!("failed");
        atomic_json(&manifest_path, &manifest)?;
        return Err(Error::new(
            "duration_limit",
            "Actual encoded durations exceed the requested ceiling",
        ));
    }
    manifest["status"] = json!("complete");
    manifest["total_duration"] = json!(total);
    manifest["processing_seconds"] = json!(started.elapsed().as_secs_f64());
    atomic_json(&manifest_path, &manifest)?;
    Ok(
        json!({"ok":true,"manifest":manifest_path.canonicalize()?,"clips":manifest["clips"],"total_duration":total,"rendered":rendered,"reused":reused,"processing_seconds":started.elapsed().as_secs_f64()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "uses supplied recording and native Windows codecs"]
    fn interrupted_batch_retains_completed_clip_and_resumes() {
        let _runtime = media::Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("video-clips-for-testing/pngtuber_speedrun.mkv");
        let output = dir.path().join("job");
        let mut job:Job=serde_json::from_value(json!({"version":2,"source":source,"output_dir":output,"settings":{"acceleration":"software","preview_height":144},"clips":[{"id":"first","range":{"start":10,"duration":0.3}},{"id":"second","range":{"start":20,"duration":20}}]})).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let manifest = output.join("manifest.json");
        let watcher = std::thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed().as_secs() < 45 {
                if let Ok(file) = File::open(&manifest) {
                    if let Ok(v) = serde_json::from_reader::<_, Value>(file) {
                        if v["clips"][0]["status"] == "complete" {
                            flag.store(true, std::sync::atomic::Ordering::SeqCst);
                            return;
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
        let err = run(job, false, &cancel).unwrap_err();
        watcher.join().unwrap();
        assert_eq!(err.code, "cancelled", "{err}");
        let v: Value =
            serde_json::from_reader(File::open(output.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(v["clips"][0]["status"], "complete");
        assert_eq!(v["status"], "cancelled");
        job=serde_json::from_value(json!({"version":2,"source":source,"output_dir":output,"settings":{"acceleration":"software","preview_height":144},"clips":[{"id":"first","range":{"start":10,"duration":0.3}},{"id":"second","range":{"start":20,"duration":0.3}}]})).unwrap();
        let result = run(job, true, &Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(result["reused"], 1);
        assert_eq!(result["rendered"], 1);
    }
}
