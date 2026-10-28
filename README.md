# AIContentEditorAssistant — native Windows

**One `acea.exe` for an AI agent to inspect recordings and render ordered edited clips.** Windows Media Foundation handles video/audio; Direct3D 11 and Direct2D handle effects; WIC writes PNGs. No FFmpeg, FFprobe, Python, GUI, AI model, or background service is required at runtime.

The calling AI makes the creative decisions. A prompt such as “Make an entertaining tutorial from this recording; choose the shortest satisfying cut, capped at 12 minutes” becomes a review → edit-list → render workflow. ACEA supplies the media operations and predictable JSON results. It does not understand the prompt, generate narration, edit CapCut projects, or assemble the final video.

## Download and run

Download the standalone **[acea.exe for Windows x64](https://github.com/captainflaskyboy-create/ContentEditorAssitant/releases/latest/download/acea.exe)** from [GitHub Releases](https://github.com/captainflaskyboy-create/ContentEditorAssitant/releases). The executable is the entire runtime; no companion files, Rust installation, FFmpeg, or Python are needed.

Save it in a folder, open PowerShell in that folder, and run:

```powershell
.\acea.exe --version
.\acea.exe doctor
.\acea.exe inspect --source "C:\Videos\recording.mp4"
.\acea.exe review --source "C:\Videos\recording.mp4" --output-dir ".\recording-review"
```

Use your own recording path and a new review output directory. Run `.\acea.exe capabilities` for the embedded AI workflow, edit-request examples, supported effects, and command reference. Optional installation is `.\acea.exe install`; open a new terminal afterward to use `acea` from any folder.

Requires Windows 10 22H2 or Windows 11 **x64**, with Windows media components and a graphics driver. Windows N editions may need the Media Feature Pack. The portable executable is unsigned.

## Developer example workflow

Windows 10 22H2 / Windows 11 x64, with Windows media components and a graphics driver. Windows N editions may need the Media Feature Pack. Copy **only `dist/acea.exe`** anywhere you like:

The following examples use the development recording, which is not included in the repository or release. Substitute your own source and timestamps when adapting them.

```powershell
.\dist\acea.exe doctor
.\dist\acea.exe capabilities
.\dist\acea.exe inspect --source ".\video-clips-for-testing\pngtuber_speedrun.mkv"
.\dist\acea.exe review --source ".\video-clips-for-testing\pngtuber_speedrun.mkv" --output-dir ".\recording-review"
```

Open the generated sheets. Each includes source timestamps; `index.json` identifies exact image rectangles and decoded times. Sample important moments more closely:

```powershell
.\dist\acea.exe frames --source ".\video-clips-for-testing\pngtuber_speedrun.mkv" --output-dir ".\detail-frames" --times "820,1350,1765"
.\dist\acea.exe validate --request examples/tutorial-job.json
.\dist\acea.exe batch --request examples/tutorial-job.json
.\dist\acea.exe batch --request examples/tutorial-job.json --resume true
```

The supplied tutorial example was chosen from the actual recording: CHARAT creation, expressions, Piskel sheet preparation, Avatar Pack Builder, and the working-avatar payoff. It renders **18 clips totaling approximately 6 minutes 10 seconds**, below the 12-minute ceiling, with a small additional AAC padding allowance. This is an example clip selection for assembly, not a narrated finished tutorial.

To compare the zoom techniques on a real UI button, run `acea batch --request examples/zoom-showcase.json --resume true`. See the [zoom showcase guide](docs/zoom-showcase.md) for the clip list and a reusable smooth button-zoom example.

## Optional command-line installation

```powershell
.\dist\acea.exe install --dry-run true
.\dist\acea.exe install
# Open a new terminal, then:
acea doctor
```

`install` copies the executable into `%LOCALAPPDATA%\AIContentEditorAssistant\bin` and appends that directory to the **user** PATH without administrator access. Existing entries and registry string type are preserved; equivalent entries are not duplicated. Running any other command never changes PATH. Use `install --replace true` to replace a different installed build. If the installed executable is running, close it before replacing it.

## Requests

Requests must explicitly specify `version:2`. File-relative paths resolve against the request JSON's directory; stdin (`--request -`) resolves against the current directory. The executable embeds its interface, examples, bounds, and agent workflow in `capabilities`.

An individual clip:

```json
{
  "version": 2,
  "source": "recording.mkv",
  "output": "edited.mp4",
  "range": {"start": 12, "duration": 6},
  "effects": [{
    "type": "camera",
    "keyframes": [
      {"time": 0, "zoom": 1},
      {"time": 0.6, "zoom": 1.8, "target": {"x": 0.7, "y": 0.35}},
      {"time": 4.8, "zoom": 1.8, "target": {"x": 0.7, "y": 0.35}},
      {"time": 6, "zoom": 1}
    ]
  }]
}
```

Run `acea validate --request clip.json`, then `acea render --request clip.json`. The output directory must exist. Existing outputs require `overwrite:true`; the source is always protected.

An ordered job:

```json
{
  "version": 2,
  "source": "recording.mkv",
  "output_dir": "tutorial-clips",
  "max_duration": 720,
  "clips": [
    {"id": "opening", "label": "Show the result", "range": {"start": 1700, "duration": 5}},
    {"id": "setup", "label": "Explain the setup", "range": {"start": 60, "duration": 12}}
  ]
}
```

`batch` creates the job directory, renders clips serially as `<id>.mp4`, and checkpoints `manifest.json` after every completed clip. The manifest records order, labels, source ranges, actual durations, cumulative timeline positions, hashes, warnings, and rendering paths. IDs are unique ignoring case, use 1–64 ASCII letters/digits/`-`/`_`, and cannot be Windows device names.

On `--resume true`, unchanged verified clips are reused. Reordering or relabeling needs no re-encoding. Source-content, effect, settings, renderer-version, and output-content changes invalidate affected clips. Unknown files in the job folder are protected. A clip-level settings object **replaces**, rather than merges with, shared job settings.

## Effects and coordinates

| Type | Required fields | Defaults |
|---|---|---|
| `zoom` | `start,end` | `from_zoom:1,to_zoom:1.5,target:center` |
| `snap_zoom` | `start,end` | `zoom:1.5,target:center` |
| `pan` | `start,end,from,to` target points | `zoom:1.5` |
| `ken_burns` | `start,end,from,to` camera states | Each state has `zoom` and optional `target` |
| `camera` | `keyframes` | Each has `time,zoom`, optional `target,easing` |
| `spotlight` | `start,end,rect` | `dim:0.65` outside rectangle |
| `blur` | `start,end,rect` | `sigma:8` source pixels |

Targets are normalized full displayed source coordinates: `(0,0)` top-left, `(1,1)` bottom-right. A target is the desired viewport center, clamped at the edges. Zoom 2 shows half the source width and height. Rectangles use `x,y,width,height` in the same space. On contact sheets, normalize **inside the cell's image rectangle**, excluding labels and neighboring cells.

Effect/keyframe times are relative to the selected source range. `frames`/`review` times are source-relative. Camera intervals are half-open `[start,end)` and return to full-frame outside them; they cannot overlap. Use one `camera` path for zoom/hold/return, repeating a state at a later time for a hold. Easing belongs to the outgoing segment: `linear`, quadratic `ease_in`, `ease_out`, or `ease_in_out` (default). Highlights run in request order **before** camera movement.

## Output, performance, and recovery

- MP4 with H.264 video and first-stream stereo 48 kHz AAC audio at 192 kbps. Silent sources stay silent; separate source audio tracks are not mixed. Audio gaps remain silence. A bounded MP4 timing-metadata shim restores leading empty audio edits omitted by the Windows source reader.
- Preserve source resolution by default, rounded to even dimensions. Output is constant-frame-rate; common approximate metadata rates are normalized to standard rational rates. Range duration rounds down to complete frames. AAC padding may add roughly 22 ms to container duration; job validation reserves this per audio clip before accepting a duration ceiling.
- Default bitrate scales from 10 Mbps at 1080p30 and is bounded to 2–40 Mbps. Override with `settings.video_bitrate_kbps`. `settings.preview_height:540` explicitly requests a smaller render.
- `settings.acceleration` defaults to `auto`; `software` forces native software codecs and WARP graphics. GPU output uses a bounded pool of tracked D3D textures when accepted. Otherwise ACEA retries with system-memory input or software rendering and reports the fallback. Decoder transform registration is not proof of active DXVA decoding; inspect reported transform and transfer information.
- Processing is serial with bounded frame buffers. `settings.threads` is retained as a work-limit hint; Windows owns codec worker pools. Performance depends on codec, GPU/driver, effects, source resolution, and competing work. See [measured results](docs/verification.md).
- One JSON result on stdout; structured JSON progress events on stderr. Exit 0: success; 2: error; 3: another job is busy; 130: cancelled. Error results identify the code and cause.
- Ctrl+C cancels the current clip, cleans its partial file, and preserves completed clips and existing final output. Resume a batch explicitly. Abrupt process termination/power loss may leave `.acea-*` temporary artifacts; remove these only when no job is active.
- `review` defaults to 15-second sampling, 320-pixel thumbnails, and 16 frames per sheet. Each call is capped at 240 samples; continue at `continuation_timestamp`. `frames` accepts at most 12 timestamps and reports actual decoded timestamps.
- Unsupported installed codecs, mid-stream dimension changes, and non-square pixels produce explicit errors. There are no automatic codec downloads or FFmpeg fallbacks.

Version 1 requests are rejected. Use version 2 and replace `settings.crf` with optional `video_bitrate_kbps`.

## Build and test

Rust 1.88+ and a matching Windows linker are needed to build, not to run. This checkout uses a workspace-local GNU Rust toolchain plus the existing MSYS2 GCC linker.

```powershell
cargo build --release --locked
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Optional media verification uses FFmpeg only to create fixtures and independently decode results. Every integration-test ACEA subprocess runs with an empty PATH:

```powershell
cargo test --locked -- --include-ignored --test-threads=1
```

The ignored cancellation/batch recovery tests use the supplied recording. In restricted environments, set `TEMP`/`TMP` to a writable workspace temporary folder. Run `scripts/package.ps1` to copy the standalone release executable into `dist`. Use `scripts/benchmark.ps1` for reproducible zoom/combined-effect measurements.

See [AI instructions](AGENTS.md), [embedded interface reference](docs/capabilities.json), and [verification](docs/verification.md). CapCut import/playback remains a separate manual check.
