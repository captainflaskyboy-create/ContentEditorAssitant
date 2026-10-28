# Native Windows verification — 28 October 2026

## Audio correction in 0.2.1

The user identified crackling in the 0.2.0 renders. A new 997 Hz tone in an AAC/MKV fixture reproduced it: the normalized adjacent-sample jump increased from 0.131 in the source to 1.209 after rendering. Millisecond-rounded source timestamps were incorrectly treated as sample gaps/overlaps at packet boundaries.

Version 0.2.1 follows the continuous PCM sample clock for timestamp jitter up to 2 ms, while preserving larger gaps and explicit source stream ticks. The regression now passes without introducing those impulses. Encoder version changes invalidate cached clips, so old renders must be regenerated. Previous test outputs were removed during cleanup; the fresh listening comparison is `test-output/basic-test/basic-cut.mp4` against `original-reference.wav` in the same directory. The test request is `examples/basic-audio-test.json` and applies no video effects.

The fresh ten-second real-recording clip was independently decoded and compared with the reference. Aligned waveform correlation was **0.999936**, with a **16-sample (0.333 ms)** alignment adjustment. This is consistent with continuous audio and ordinary AAC re-encoding differences; the listening comparison remains available for subjective verification. Cleanup preserved the source recording SHA-256 `CA14F2F77A89A2E8FFE8623FB27A469B58F6AA8EEAB1887E52A801F33EFE9FFD`.

The measurements and tutorial exercise below describe the earlier 0.2.0 build; they are historical, not a claim that its audio was artifact-free.

ACEA 0.2.0 replaces all runtime FFmpeg/FFprobe operations with Windows APIs. The package contains one `acea.exe`, **824,832 bytes (approximately 806 KiB)**. Rust dependencies are linked into it; inspected executable imports reference Windows system DLLs, with no Rust, Python, FFmpeg, or MinGW runtime DLL requirement.

## Functional checks

- Formatter and Clippy with warnings denied.
- **18 passing tests** covering request validation and migration; camera overlap/coordinates; real rendered geometry; all four easing curves; Ken Burns and general camera paths; overlapping spotlight/blur/zoom; image blur variance; source audio offsets and nonzero seeks; silent output; right-angle rotation; 30/60 fps output; PNG sampling with actual timestamps; review sheets; previews; native software rendering/encoding; protected overwrite; native encoder error cleanup; render locking; running-render cancellation; interrupted-batch resume; ordering; duration ceilings; duplicate IDs; changed edits/source content; and corrupted output recovery.
- Media integration fixtures and independent output checks use FFmpeg as a development oracle. **Every tested ACEA subprocess runs with an empty PATH.** No FFmpeg/FFprobe executable is beside the test binary or the portable release.
- A copy of the release executable runs from `test-output/portable path`, with no neighboring assets and an empty PATH. `doctor` succeeds. Executable import inspection shows only Windows-provided DLL dependencies.
- Installation copy/replacement and PATH de-duplication are tested against temporary files and strings. `install --dry-run true` succeeds without copying files or changing PATH. No live user PATH was changed during verification.
- A direct review-cap check sampled exactly 240 frames, returned continuation timestamp 240 seconds, and a follow-up request sampled the one remaining frame in the selected 241-second range.
- Cancellation tests exercise the same atomic flag used by the console handler, terminate an in-progress native render, verify cleanup, preserve existing output, and resume a batch while reusing completed clips. An interactive console Ctrl+C keypress has not been manually exercised.

Run the complete checks from the repository root, with Rust and the development FFmpeg oracle on PATH:

```powershell
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --include-ignored --test-threads=1
```

The ignored cancellation/batch tests use the supplied recording. Restricted environments may require TEMP/TMP to point to a writable workspace temporary folder. Ordinary `cargo test` excludes tests requiring the recording or development oracle.

## Performance comparison

Release builds, six-second ranges, source resolution. Native graphics reports **Intel Iris Xe**, and the active native encoder transform advertises hardware encoding. Both the 720p and 1080p60 sources supplied compatible decoder textures for every output frame, and encoding used the bounded D3D11 texture pool with no ACEA CPU frame readback. This does not independently establish the decoder's internal DXVA implementation.

Combined effects means zoom, spotlight dimming, and rectangular sidebar blur. Times include probing and output verification inside ACEA; process startup adds a small amount. Historical CPU FFmpeg results are retained in [the baseline report](verification-ffmpeg-baseline.md).

| Input | Effects | FFmpeg CPU seconds | Native auto seconds | FFmpeg sampled working set | Native sampled working set |
|---|---|---:|---:|---:|---:|
| Recording, 1280×720 / 30 fps | Zoom | 1.736 | 2.524 | 165.2 MiB | 264.2 MiB |
| Recording, 1280×720 / 30 fps | Combined | 2.793 | 2.497 | 166.4 MiB | 289.2 MiB |
| Synthetic motion, 1920×1080 / 60 fps | Zoom | 6.749 | 3.746 | 284.0 MiB | 375.8 MiB |
| Synthetic motion, 1920×1080 / 60 fps | Combined | 13.342 | 4.119 | 317.2 MiB | 390.6 MiB |

Forced native software on the 720p recording took 6.914 seconds for zoom and 7.239 seconds for combined effects, with sampled working sets of 178.6 and 189.8 MiB. WARP graphics, software encoder transforms, and system-memory transfer were reported as expected.

The native backend is **not universally faster or lower-memory**. It removes roughly 201 MiB of bundled video tools from the runtime distribution, but GPU initialization and native media components raise the measured process working set here. The larger 1080p60 effects job benefited most from GPU execution.

Memory is a 40 ms sampled working-set peak. Native measurements include the ACEA process, while historical measurements summed ACEA and its FFmpeg/FFprobe children. Shared pages are included; GPU memory and out-of-process driver/service allocations are excluded. These are short observations on this PC, not a full-video time or total-memory guarantee.

Encoding quality is not held constant across backends: FFmpeg used CRF 18/veryfast; native output uses resolution/frame-rate-based bitrate and a hardware encoder. Camera interpolation and blur kernels also differ. Native geometry/easing and blur behavior pass decoded-image tests, and reviewed UI text remains legible; these measurements do not establish identical compression quality or pixel output.

Raw reports: `test-output/benchmark-native/report.json`, `test-output/benchmark-native-1080p60/report.json`, and `test-output/benchmark-native-software/report.json`. Reproduce using `scripts/benchmark.ps1`.

## Full tutorial workflow

- Reviewed the complete **1782.566-second recording** through 119 timestamped samples on eight contact sheets, followed by detailed inspection of expressions, slicing controls, pack assignment, import, and the final desktop avatar.
- Created `examples/tutorial-job.json`: 18 ordered clips, including a payoff opening, CHARAT creation, expression preparation, Piskel sheet work, Avatar Pack Builder setup, export/import, and the working-avatar payoff. Long browsing and experimentation were omitted.
- Native release rendered all clips in **73.14 seconds**, with **370.176 seconds (6:10.176)** total actual container duration, below the 720-second ceiling. Files and manifest are in `test-output/tutorial-clips-native`.
- A second run verified and reused **all 18 clips**, rendering none, in **5.46 seconds**.
- Inspected rendered opening, enlarged slice-grid controls, and payoff frames. The grid labels and values remain legible; target framing and full-frame returns match the selected effects.

The result is a labeled clip set for assembly, not a narrated or assembled finished tutorial. The example illustrates an editorial selection; it does not claim mathematical proof of the shortest possible satisfying cut.

## Remaining manual acceptance

CapCut was not launched. Import/playback in the intended CapCut version remains unverified. Real installation into the user's normal profile was intentionally not performed; its copy/PATH logic is covered by isolated tests and a non-mutating dry run. Windows driver and codec availability can differ on another PC. Abrupt termination/power loss may leave temporary files rather than clean cancellation.
