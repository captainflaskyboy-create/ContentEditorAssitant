mod batch;
mod encoder;
mod error;
mod graphics;
mod install;
mod media;
mod model;
mod mp4;
mod render;
mod review;
mod validate;

use error::{Error, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

fn required<'a>(args: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str> {
    args.get(name).map(String::as_str).ok_or_else(|| {
        Error::new(
            "invalid_arguments",
            format!("Missing --{name}; run acea help"),
        )
    })
}

fn options(raw: &[String], allowed: &[&str]) -> Result<BTreeMap<String, String>> {
    if !raw.len().is_multiple_of(2) {
        return Err(Error::new(
            "invalid_arguments",
            "Options require values; run acea help",
        ));
    }
    let mut result = BTreeMap::new();
    for pair in raw.as_chunks::<2>().0 {
        let key = pair[0]
            .strip_prefix("--")
            .ok_or_else(|| Error::new("invalid_arguments", "Expected a named --option"))?;
        if !allowed.contains(&key) || result.contains_key(key) {
            return Err(Error::new(
                "invalid_arguments",
                format!("Unknown or repeated option: {}", pair[0]),
            ));
        }
        result.insert(key.to_string(), pair[1].clone());
    }
    Ok(result)
}

fn execute(cancelled: &Arc<AtomicBool>) -> Result<Value> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let verb = raw.first().map(String::as_str).unwrap_or("help");
    let rest = raw.get(1..).unwrap_or_default();
    let _runtime = if matches!(
        verb,
        "doctor" | "inspect" | "frames" | "validate" | "render" | "review" | "batch"
    ) {
        Some(media::Runtime::new()?)
    } else {
        None
    };
    match verb {
        "help" | "--help" | "-h" => {
            options(rest, &[])?;
            Ok(
                json!({"ok":true,"name":"AIContentEditorAssistant","version":env!("CARGO_PKG_VERSION"),"commands":[
                "acea doctor", "acea capabilities", "acea inspect --source <video>",
                "acea frames --source <video> --output-dir <new-directory> --times <0,5,10> [--height <540>]",
                "acea validate --request <file.json|->", "acea render --request <file.json|->"
                ,"acea review --source <video> --output-dir <new-directory> [--start <0>] [--duration <seconds>] [--interval <15>]"
                ,"acea batch --request <job.json|-> [--resume <false|true>]"
                ,"acea install [--replace <false|true>] [--dry-run <false|true>]"
            ],"notes":["All commands return JSON on stdout; render progress goes to stderr.","Request file paths resolve relative to the JSON file. With stdin, paths resolve relative to the working directory.","Frames timestamps refer to the source. Effect timestamps refer to the selected range."]}),
            )
        }
        "--version" => {
            options(rest, &[])?;
            Ok(json!({"ok":true,"version":env!("CARGO_PKG_VERSION")}))
        }
        "doctor" => {
            options(rest, &[])?;
            media::doctor()
        }
        "capabilities" => {
            options(rest, &[])?;
            Ok(serde_json::from_str(include_str!(
                "../docs/capabilities.json"
            ))?)
        }
        "inspect" => {
            let args = options(rest, &["source"])?;
            Ok(json!({"ok":true,"media":media::inspect(required(&args,"source")?.as_ref())?}))
        }
        "validate" | "render" => {
            let args = options(rest, &["request"])?;
            if verb == "validate" {
                let (path, _base) = render::load_json(required(&args, "request")?)?;
                if path.get("clips").is_some() {
                    let job = batch::from_value(path, &_base)?;
                    let (m, clips) = batch::validate_job(&job)?;
                    return Ok(
                        json!({"ok":true,"valid":true,"media":m,"clips":clips,"total_duration":clips.iter().map(|c|c["duration"].as_f64().unwrap()).sum::<f64>()}),
                    );
                }
                // Stdin has already been consumed, so deserialize the loaded object directly.
                if required(&args, "request")? == "-" {
                    let mut req: model::Request = serde_json::from_value(path)?;
                    if req.source.is_relative() {
                        req.source = _base.join(req.source);
                    }
                    if req.output.is_relative() {
                        req.output = _base.join(req.output);
                    }
                    let (m, warnings) = render::validate_request(&req)?;
                    let (w, h) = graphics::dimensions(&req, &m);
                    return Ok(
                        json!({"ok":true,"valid":true,"media":m,"output":{"path":req.output,"width":w,"height":h},"warnings":warnings}),
                    );
                }
            }
            let req = render::load(required(&args, "request")?)?;
            if verb == "render" {
                render::render(req, cancelled)
            } else {
                let (media, warnings) = render::validate_request(&req)?;
                let (width, height) = graphics::dimensions(&req, &media);
                Ok(
                    json!({"ok":true,"valid":true,"media":media,"output":{"path":req.output,"width":width,"height":height,"duration":req.range.duration},"warnings":warnings}),
                )
            }
        }
        "batch" => {
            let args = options(rest, &["request", "resume"])?;
            let resume = parse_bool(args.get("resume"))?;
            batch::run(batch::load(required(&args, "request")?)?, resume, cancelled)
        }
        "review" => {
            let args = options(
                rest,
                &["source", "output-dir", "start", "duration", "interval"],
            )?;
            let parse = |key: &str, default: f64| -> Result<f64> {
                args.get(key).map_or(Ok(default), |v| {
                    v.parse::<f64>().map_err(|_| {
                        Error::new("invalid_arguments", format!("--{key} must be seconds"))
                    })
                })
            };
            review::review(
                required(&args, "source")?.into(),
                required(&args, "output-dir")?.into(),
                parse("start", 0.0)?,
                if args.contains_key("duration") {
                    Some(parse("duration", 0.0)?)
                } else {
                    None
                },
                parse("interval", 15.0)?,
                cancelled,
            )
        }
        "install" => {
            let args = options(rest, &["replace", "dry-run"])?;
            install::install(
                parse_bool(args.get("replace"))?,
                parse_bool(args.get("dry-run"))?,
            )
        }
        "frames" => {
            let args = options(rest, &["source", "output-dir", "times", "height"])?;
            let times = required(&args, "times")?
                .split(',')
                .map(|s| {
                    s.trim().parse::<f64>().map_err(|_| {
                        Error::new(
                            "invalid_arguments",
                            "--times must be comma-separated seconds",
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let height = args
                .get("height")
                .map(|s| s.parse::<u32>())
                .transpose()
                .map_err(|_| Error::new("invalid_arguments", "--height must be an integer"))?
                .unwrap_or(540);
            render::frames(
                required(&args, "source")?.into(),
                required(&args, "output-dir")?.into(),
                times,
                height,
                cancelled,
            )
        }
        _ => Err(Error::new(
            "invalid_arguments",
            format!("Unknown command: {verb}; run acea help"),
        )),
    }
}

fn parse_bool(value: Option<&String>) -> Result<bool> {
    match value.map(String::as_str) {
        None | Some("false") => Ok(false),
        Some("true") => Ok(true),
        _ => Err(Error::new(
            "invalid_arguments",
            "Boolean options require true or false",
        )),
    }
}

fn main() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        signal.store(true, Ordering::SeqCst);
    }) {
        println!(
            "{}",
            Error::new("signal_handler_failed", e.to_string()).json()
        );
        std::process::exit(2);
    }
    match execute(&cancelled) {
        Ok(value) => {
            let ok = value["ok"] == true;
            println!("{}", value);
            if !ok {
                std::process::exit(2);
            }
        }
        Err(e) => {
            println!("{}", e.json());
            std::process::exit(match e.code {
                "cancelled" => 130,
                "busy" => 3,
                _ => 2,
            });
        }
    }
}
