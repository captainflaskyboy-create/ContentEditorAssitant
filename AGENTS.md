# ACEA: native Windows tool for AI editors

## Anonymous repository identity

This repository belongs to `captainflaskyboy-create`. Before every commit, amend, tag, or push, explicitly verify both author and committer use `Captain Flasky Boy` and `337722320+captainflaskyboy-create@users.noreply.github.com`. Configure this identity locally for this repository; never inherit a personal global identity. Do not add personal co-author trailers, signatures, contact details, or identifying local paths. Audit all commits being published and all tracked files for personal identity before pushing.

Build a small Rust command-line tool whose runtime uses Windows media/graphics APIs. No FFmpeg/FFprobe subprocesses, Python runtime, GUI, local inference models, or background services. Stream bounded frame buffers; keep rendering serial. FFmpeg may be used as a **development-only** fixture generator and independent verification oracle.

## Agent workflow

1. Run `acea doctor` and `acea capabilities`. The executable embeds the entire supported interface and workflow. Do not infer active acceleration solely from registered hardware or GPU presence.
2. Inspect the source, then call `review` for timestamped contact sheets. Open the actual generated images. Continue at `continuation_timestamp` if capped; review narrower ranges or call `frames` for important moments.
3. Establish the story from the recording. For a tutorial, identify setup, essential actions, expressions/assets, configuration, and the working result. Cut redundant browsing and experimentation; the user's duration cap is a ceiling, not a target.
4. Choose normalized full-source targets/rectangles. When reading a contact sheet, normalize within the cell's image_rect, excluding labels. Sampling timestamps are source-relative; effect/keyframe timestamps are relative to each selected range.
5. Write an explicit version 2 job with stable IDs, labels, ordered ranges, precise effects, and the user's duration ceiling. Relative paths resolve against the request file directory (cwd for stdin). Optional clip settings replace shared settings.
6. Validate the complete job before rendering. Resolve errors and review warnings. Do not ask the renderer to make unsupported creative decisions.
7. Run `batch`. Parse the single stdout JSON; stderr contains structured progress. On failure/cancellation, inspect the checkpoint and use `--resume true`. Unchanged verified clips are reused; changed edits/content are rerendered. Keep the source intact.
8. Inspect rendered opening, instructional targets, and payoff frames. Return the ordered clip paths and manifest for assembly. ACEA does not assemble the final video, generate voiceover, or modify CapCut. Do not claim that CapCut import/playback was verified unless it was.

## Editing semantics

Use one camera path for a continuous zoom/hold/return or externally supplied tracking coordinates. Keyframe easing belongs to the outgoing segment. Camera intervals are half-open and return to full-frame outside them. Camera overlaps are rejected. Highlights run in list order before the camera; Gaussian blur draws within the target rectangle. Keep effects purposeful and UI text legible.

Output is H.264/AAC MP4, source resolution by default. Optional preview_height reduces resolution. Bitrate replaces CRF in version 2. Native AAC may add a short padding tail; validation reserves a packet per clip in capped jobs. Only the first audio stream is used.

## Recovery and development

- `busy`/exit 3: retry after the active job, without concurrent render workers.
- `invalid_request`: correct the specified fields. Version 1 and unknown fields are rejected.
- `unsupported_media`/`media_unavailable`: diagnose native codec availability; do not download codecs or add an FFmpeg fallback silently.
- GPU failure: the native renderer can retry with a bounded system-memory path or software rendering. Surface-transfer and transform reports determine what actually happened.
- `cancelled`/exit 130: existing final files and completed clips survive. Resume explicitly.
- Installation is explicit: only `install` may copy to the per-user tools folder and modify the user PATH. Test copy/PATH logic in isolation; do not alter the user's real PATH during tests.
- Run formatter, Clippy, unit tests, and appropriate native integration tests. Keep source recordings, toolchains, generated clips, and distribution binaries out of Git. Do not spawn parallel agents unless the user explicitly requests delegation.
