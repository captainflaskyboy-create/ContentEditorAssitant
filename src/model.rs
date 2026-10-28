use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn version() -> u32 {
    2
}
fn threads() -> u32 {
    2
}
fn zoom() -> f64 {
    1.5
}
fn dim() -> f64 {
    0.65
}
fn sigma() -> f64 {
    8.0
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default = "version")]
    pub version: u32,
    pub source: PathBuf,
    pub output: PathBuf,
    pub range: Range,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub start: f64,
    pub duration: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "threads")]
    pub threads: u32,
    #[serde(default)]
    pub video_bitrate_kbps: Option<u32>,
    #[serde(default)]
    pub acceleration: Acceleration,
    #[serde(default)]
    pub preview_height: Option<u32>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            threads: threads(),
            video_bitrate_kbps: None,
            acceleration: Acceleration::Auto,
            preview_height: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Acceleration {
    #[default]
    Auto,
    Software,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Default for Point {
    fn default() -> Self {
        Self { x: 0.5, y: 0.5 }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    #[default]
    EaseInOut,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CameraState {
    pub zoom: f64,
    #[serde(default)]
    pub target: Point,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Keyframe {
    pub time: f64,
    pub zoom: f64,
    #[serde(default)]
    pub target: Point,
    /// Easing from this keyframe to the next.
    #[serde(default)]
    pub easing: Easing,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    Zoom {
        start: f64,
        end: f64,
        #[serde(default)]
        target: Point,
        #[serde(default = "one")]
        from_zoom: f64,
        #[serde(default = "zoom")]
        to_zoom: f64,
        #[serde(default)]
        easing: Easing,
    },
    SnapZoom {
        start: f64,
        end: f64,
        #[serde(default)]
        target: Point,
        #[serde(default = "zoom")]
        zoom: f64,
    },
    Pan {
        start: f64,
        end: f64,
        from: Point,
        to: Point,
        #[serde(default = "zoom")]
        zoom: f64,
        #[serde(default)]
        easing: Easing,
    },
    KenBurns {
        start: f64,
        end: f64,
        from: CameraState,
        to: CameraState,
        #[serde(default)]
        easing: Easing,
    },
    Camera {
        keyframes: Vec<Keyframe>,
    },
    Spotlight {
        start: f64,
        end: f64,
        rect: Rect,
        #[serde(default = "dim")]
        dim: f64,
    },
    Blur {
        start: f64,
        end: f64,
        rect: Rect,
        #[serde(default = "sigma")]
        sigma: f64,
    },
}
fn one() -> f64 {
    1.0
}

impl Effect {
    pub fn interval(&self) -> (f64, f64) {
        match self {
            Self::Zoom { start, end, .. }
            | Self::SnapZoom { start, end, .. }
            | Self::Pan { start, end, .. }
            | Self::KenBurns { start, end, .. }
            | Self::Spotlight { start, end, .. }
            | Self::Blur { start, end, .. } => (*start, *end),
            Self::Camera { keyframes } => (
                keyframes.first().map_or(0.0, |k| k.time),
                keyframes.last().map_or(0.0, |k| k.time),
            ),
        }
    }
    pub fn camera(&self) -> bool {
        !matches!(self, Self::Spotlight { .. } | Self::Blur { .. })
    }
    pub fn keyframes(&self) -> Vec<Keyframe> {
        let make = |time, zoom, target, easing| Keyframe {
            time,
            zoom,
            target,
            easing,
        };
        match self {
            Self::Zoom {
                start,
                end,
                target,
                from_zoom,
                to_zoom,
                easing,
            } => vec![
                make(*start, *from_zoom, *target, *easing),
                make(*end, *to_zoom, *target, *easing),
            ],
            Self::SnapZoom {
                start,
                end,
                target,
                zoom,
            } => vec![
                make(*start, *zoom, *target, Easing::Linear),
                make(*end, *zoom, *target, Easing::Linear),
            ],
            Self::Pan {
                start,
                end,
                from,
                to,
                zoom,
                easing,
            } => vec![
                make(*start, *zoom, *from, *easing),
                make(*end, *zoom, *to, *easing),
            ],
            Self::KenBurns {
                start,
                end,
                from,
                to,
                easing,
            } => vec![
                make(*start, from.zoom, from.target, *easing),
                make(*end, to.zoom, to.target, *easing),
            ],
            Self::Camera { keyframes } => keyframes.clone(),
            _ => vec![],
        }
    }
}
