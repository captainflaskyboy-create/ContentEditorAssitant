# Verification — 28 October 2026

Built and exercised on Windows with Rust 1.99.1 (GNU toolchain) and FFmpeg 9.0.2 essentials from gyan.dev. The runtime needs no Rust installation; the compiled `acea.exe` is 660,992 bytes (about 646 KiB). The two general-purpose FFmpeg executables add approximately 201 MiB on disk; an existing FFmpeg installation may be used instead.

## Checks

- Rust formatter and Clippy with warnings denied.
- 13 passing tests: input and coordinate validation; camera overlap handling; real rendered target geometry; all four easing curves; Ken Burns and camera keyframes; overlapping spotlight/blur/zoom; blur variance; silent output; duration; nonzero source seek and delayed audio; frame extraction; preview sizing; Unicode/spaced paths; missing dependencies; render locking; cancellation of a running encoder; rejected and successful overwrite; and preservation of existing output after encoder failure.
- Rendered `examples/zoom-focus.json` against the provided recording. Inspected frames at 0, 2, and 5.8 seconds: full frame, enlarged character with aligned rectangular spotlight, then full-frame return. Output has 1280×720 H.264 video, 30 fps, six-second duration, and AAC audio.
- Packaged `dist/acea.exe` locates FFmpeg/FFprobe beside itself and passes `doctor` without PATH changes.

Run checks from the repository root with Rust and FFmpeg on PATH:

```powershell
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --include-ignored --test-threads=1
```

Media tests are opt-in/ignored by default so ordinary `cargo test` works without FFmpeg. In a restricted environment that does not allow temporary-file moves in the default temporary folder, set `TEMP` and `TMP` to a writable workspace temporary directory before running media tests.

## Measured performance

Default two threads per processing stage, CRF 18, `veryfast` encoding, source resolution, one job at a time. Each input range is six seconds. Combined effects means zoom plus a rectangular spotlight and a blurred sidebar. Times include source probing, encoding, and output probing, measured inside ACEA; process startup/polling adds a little overhead.

| Input | Effects | Render seconds | Media seconds / render second | Sampled peak working set |
|---|---|---:|---:|---:|
| Provided screen recording, 1280×720, 30 fps | Zoom | 1.736 | 3.46× | 165.2 MiB |
| Provided screen recording, 1280×720, 30 fps | Combined | 2.793 | 2.15× | 166.4 MiB |
| Synthetic moving test pattern, 1920×1080, 60 fps | Zoom | 6.749 | 0.89× | 284.0 MiB |
| Synthetic moving test pattern, 1920×1080, 60 fps | Combined | 13.342 | 0.45× | 317.2 MiB |

Memory is the peak of 40 ms samples of working sets for ACEA and newly started FFmpeg/FFprobe processes. Shared pages may be counted more than once and brief peaks may be missed. No hardware identity was available through the restricted Windows query. These short measurements are observations for this environment, not a full-video timing guarantee. Source codec, motion, effect count, storage, and CPU contention affect results.

Reproduce the recording measurements with `scripts/benchmark.ps1`. Raw results and clips are in ignored `test-output/benchmark` and `test-output/benchmark-1080p60` folders. The synthetic input was generated using FFmpeg's `testsrc2` at 1920×1080, 60 fps, six seconds, encoded as H.264 with CRF 18 and the `veryfast` preset.

## Remaining manual acceptance

CapCut was not launched. Import/playback in the intended CapCut version remains unverified. The outputs use conventional H.264/yuv420p and AAC in MP4, and their streams, timing, and decoded frames were checked with FFmpeg/FFprobe.

The cancellation test exercises the application's running-child cancellation mechanism; an interactive Windows console Ctrl+C event has not been manually exercised. Force-kill and power-loss recovery are limited to leaving discoverable temporary files rather than guaranteed cleanup.
