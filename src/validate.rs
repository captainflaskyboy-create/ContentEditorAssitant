use crate::{
    error::{Error, Result},
    media::Media,
    model::*,
};
use std::path::Path;

fn invalid(message: impl Into<String>) -> Error {
    Error::new("invalid_request", message)
}
fn bounded(v: f64, low: f64, high: f64, label: &str) -> Result<()> {
    if !v.is_finite() || v < low || v > high {
        return Err(invalid(format!("{label} must be between {low} and {high}")));
    }
    Ok(())
}
fn point(p: Point) -> Result<()> {
    bounded(p.x, 0.0, 1.0, "target.x")?;
    bounded(p.y, 0.0, 1.0, "target.y")
}
fn rect(r: Rect, m: &Media) -> Result<()> {
    bounded(r.x, 0.0, 1.0, "rect.x")?;
    bounded(r.y, 0.0, 1.0, "rect.y")?;
    bounded(r.width, 0.0, 1.0, "rect.width")?;
    bounded(r.height, 0.0, 1.0, "rect.height")?;
    if r.width * (m.width as f64) < 2.0
        || r.height * (m.height as f64) < 2.0
        || r.x + r.width > 1.0 + 1e-9
        || r.y + r.height > 1.0 + 1e-9
    {
        return Err(invalid(
            "Rectangles must be inside the source and at least 2 pixels wide and high",
        ));
    }
    Ok(())
}

pub fn validate(req: &Request, m: &Media) -> Result<Vec<String>> {
    validate_with_output(req, m, true)
}
pub fn validate_with_output(req: &Request, m: &Media, check_output: bool) -> Result<Vec<String>> {
    if req.version != 2 {
        return Err(invalid("Native ACEA requires version 2. Set version:2 and replace settings.crf with optional video_bitrate_kbps; see acea capabilities"));
    }
    bounded(req.range.start, 0.0, m.duration, "range.start")?;
    bounded(req.range.duration, 0.001, m.duration, "range.duration")?;
    if req.range.start + req.range.duration > m.duration + 1e-6 {
        return Err(invalid("Selected range extends beyond the source duration"));
    }
    if req.range.duration * m.fps < 1.0 {
        return Err(invalid("Selected range must contain at least one frame"));
    }
    if req.effects.len() > 32 {
        return Err(invalid(
            "At most 32 effects are allowed per request; split larger jobs into clips",
        ));
    }
    if !(1..=16).contains(&req.settings.threads) {
        return Err(invalid("settings.threads must be between 1 and 16"));
    }
    if req
        .settings
        .video_bitrate_kbps
        .is_some_and(|b| !(100..=100_000).contains(&b))
    {
        return Err(invalid(
            "settings.video_bitrate_kbps must be between 100 and 100000",
        ));
    }
    if let Some(h) = req.settings.preview_height {
        if !(144..=m.height).contains(&h) || h % 2 != 0 {
            return Err(invalid(
                "preview_height must be even, at least 144, and no larger than source height",
            ));
        }
    }
    if m.fps > 120.0 {
        return Err(invalid(
            "Frame rates above 120 fps are outside this release's supported range",
        ));
    }
    if !["1:1", "N/A", "0:1"].contains(&m.sample_aspect_ratio.as_str()) {
        return Err(invalid("Non-square source pixels are not supported; normalize the source to square pixels first"));
    }
    if m.rotation.rem_euclid(90) != 0 {
        return Err(invalid("Source rotation must be a multiple of 90 degrees"));
    }
    if req
        .output
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("mp4"))
        != Some(true)
    {
        return Err(invalid("Output must use the .mp4 extension"));
    }
    if check_output {
        let parent = req
            .output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if !parent.is_dir() {
            return Err(invalid("Output directory must already exist"));
        }
        let target = parent.canonicalize()?.join(
            req.output
                .file_name()
                .ok_or_else(|| invalid("Output filename is required"))?,
        );
        if target == m.source || req.output.canonicalize().ok().as_ref() == Some(&m.source) {
            return Err(invalid("Output must not replace the source"));
        }
        if req.output.exists() && !req.overwrite {
            return Err(Error::new(
                "output_exists",
                "Output already exists; choose another path or set overwrite:true",
            ));
        }
        if req.output.exists() && !req.output.is_file() {
            return Err(invalid("Output path must be a regular file"));
        }
    }
    let mut intervals = vec![];
    let mut segments = 0;
    for (i, e) in req.effects.iter().enumerate() {
        let (start, end) = e.interval();
        bounded(
            start,
            0.0,
            req.range.duration,
            &format!("effects[{i}].start"),
        )?;
        bounded(end, 0.0, req.range.duration, &format!("effects[{i}].end"))?;
        if end <= start {
            return Err(invalid(format!("effects[{i}] must end after it starts")));
        }
        if e.camera() {
            let keys = e.keyframes();
            if keys.len() < 2 || keys.len() > 32 {
                return Err(invalid("Camera effects require between 2 and 32 keyframes"));
            }
            segments += keys.len() - 1;
            if segments > 64 {
                return Err(invalid("At most 64 camera segments are allowed per request; split larger jobs into clips"));
            }
            for (j, k) in keys.iter().enumerate() {
                bounded(k.time, 0.0, req.range.duration, "keyframe.time")?;
                bounded(k.zoom, 1.0, 10.0, "zoom")?;
                point(k.target)?;
                if j > 0 && k.time <= keys[j - 1].time {
                    return Err(invalid("Camera keyframe times must be strictly increasing"));
                }
            }
            intervals.push((start, end));
        } else {
            match e {
                Effect::Spotlight { rect: r, dim, .. } => {
                    rect(*r, m)?;
                    bounded(*dim, 0.0, 1.0, "dim")?;
                }
                Effect::Blur { rect: r, sigma, .. } => {
                    rect(*r, m)?;
                    bounded(*sigma, 0.1, 50.0, "sigma")?;
                }
                _ => unreachable!(),
            }
        }
    }
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    if intervals.windows(2).any(|w| w[1].0 < w[0].1) {
        return Err(invalid("Camera effects overlap. Combine them into one camera keyframe path or use non-overlapping intervals"));
    }
    let mut warnings = vec![];
    if req.settings.preview_height.is_some() {
        warnings.push("Preview resolution requested; output is smaller than the source".into());
    }
    if !m.width.is_multiple_of(2) || !m.height.is_multiple_of(2) {
        warnings.push("Odd source dimensions rounded down to even dimensions for H.264".into());
    }
    if !intervals.is_empty() {
        warnings.push("Camera intervals are half-open [start,end); framing returns to the full image outside them. Add hold/return keyframes within a camera effect for a continuous move".into());
    }
    if m.audio_streams.len() > 1 {
        warnings.push("Only the first audio stream is included; mix separate recording tracks before rendering if needed".into());
    }
    warnings.push("Output uses a constant frame rate based on source metadata; variable-rate sources are resampled".into());
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Request, Media, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fixture").unwrap();
        let source = source.canonicalize().unwrap();
        let m = Media {
            source: source.clone(),
            width: 1920,
            height: 1080,
            duration: 30.0,
            frame_rate: "60/1".into(),
            fps: 60.0,
            has_audio: false,
            audio_streams: vec![],
            sample_aspect_ratio: "1:1".into(),
            rotation: 0,
        };
        let req = Request {
            version: 2,
            source,
            output: dir.path().join("out.mp4"),
            range: Range {
                start: 5.0,
                duration: 10.0,
            },
            effects: vec![],
            settings: Settings::default(),
            overwrite: false,
        };
        (req, m, dir)
    }
    #[test]
    fn reject_bad_range_and_source_overwrite() {
        let (mut r, m, _d) = fixture();
        r.range.start = 25.0;
        assert!(validate(&r, &m).is_err());
        r.range.start = 0.0;
        r.output = r.source.clone();
        assert!(validate(&r, &m).is_err());
    }
    #[test]
    fn cameras_cannot_overlap_but_highlights_can() {
        let (mut r, m, _d) = fixture();
        let snap = |start, end| Effect::SnapZoom {
            start,
            end,
            target: Point::default(),
            zoom: 1.5,
        };
        r.effects = vec![snap(0.0, 4.0), snap(3.0, 5.0)];
        assert!(validate(&r, &m).is_err());
        r.effects = vec![
            snap(0.0, 4.0),
            snap(4.0, 5.0),
            Effect::Spotlight {
                start: 0.0,
                end: 8.0,
                rect: Rect {
                    x: 0.1,
                    y: 0.1,
                    width: 0.2,
                    height: 0.2,
                },
                dim: 0.5,
            },
        ];
        assert!(validate(&r, &m).is_ok());
    }
    #[test]
    fn rejects_bad_coordinates_and_duplicate_keyframes() {
        let (mut r, m, _d) = fixture();
        r.effects = vec![Effect::Blur {
            start: 0.0,
            end: 1.0,
            rect: Rect {
                x: 0.9,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            },
            sigma: 8.0,
        }];
        assert!(validate(&r, &m).is_err());
        let k = Keyframe {
            time: 0.0,
            zoom: 2.0,
            target: Point::default(),
            easing: Easing::Linear,
        };
        r.effects = vec![Effect::Camera {
            keyframes: vec![k, k],
        }];
        assert!(validate(&r, &m).is_err());
    }
    #[test]
    fn rejects_unknown_json_fields() {
        let raw = r#"{"source":"a","output":"b","range":{"start":0,"duration":1},"effects":[{"type":"snap_zoom","start":0,"end":1,"zom":2}]}"#;
        assert!(serde_json::from_str::<Request>(raw).is_err());
    }
}
