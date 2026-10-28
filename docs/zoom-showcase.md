# Compare every supported zoom technique

The command below generates comparison clips in `test-output/zoom-showcase`. After rendering, open the numbered MP4 files in order, or compare individual clips with `00-original.mp4`. Each is eight seconds at the recording's original resolution, with source audio.

Run or repeat the showcase from the project folder:

```powershell
.\dist\acea.exe batch --request examples/zoom-showcase.json --resume true
```

Unchanged clips are verified and reused. Edit the JSON to try different amounts, targets, timing, or easing; rerunning updates only affected clips. To render everything into a fresh folder, change `output_dir` in a copy of the request.

| Clip | What to look for |
|---|---|
| `00-original` | Original page for comparison |
| `01-smooth-linear` | Constant-speed zoom, hold, and return |
| `02-smooth-ease-in` | Gentle start, accelerating into the target |
| `03-smooth-ease-out` | Fast start, slowing as it reaches the target |
| `04-smooth-ease-in-out` | Gentle start and finish; a useful default for tutorial UI |
| `05-snap-zoom` | Immediate 2.4× emphasis, then an immediate return |
| `06-ken-burns` | Gradual zoom and pan together |
| `07-pan-to-button` | Camera moves from the character toward the button at fixed zoom |
| `08-zoom-and-spotlight` | Smooth zoom plus dimming around the target button during the hold |
| `09-zoom-and-blur` | Smooth zoom with a blurred panel below the button |
| `10-zoom-out` | Start on the button, then pull back to show context |
| `11-edge-button` | Target near the left edge; the viewport clamps to keep the image inside the frame |
| `12-subtle-punch-in` | A modest 15% snap zoom for emphasis |

The main target is CHARAT's pink **Done** button in source seconds **880–888** (14:40–14:48). The edge example targets Avatar Pack Builder's **Apply Slice Grid** button in **1385–1393** (23:05–23:13). These coordinates are specific to this recording.

## Adapt the smooth button zoom

The smooth examples use a `camera` path: full view until 0.7 seconds; zoom in over two seconds; hold until 5.5 seconds; return by 7.2 seconds; finish on the full page. The following effect can be placed in any eight-second clip request:

```json
{
  "type": "camera",
  "keyframes": [
    {"time": 0, "zoom": 1},
    {"time": 0.7, "zoom": 1, "easing": "ease_in_out"},
    {"time": 2.7, "zoom": 2.4, "target": {"x": 0.488, "y": 0.206}},
    {"time": 5.5, "zoom": 2.4, "target": {"x": 0.488, "y": 0.206}, "easing": "ease_in_out"},
    {"time": 7.2, "zoom": 1},
    {"time": 8, "zoom": 1}
  ]
}
```

Change `target` to the normalized center of your button: `x = button_center_x / full_image_width`, `y = button_center_y / full_image_height`. Use coordinates measured on the original full frame, before zoom. Increase `zoom` for a tighter crop, and allow enough hold time to read the control. Easing applies from a keyframe to the next one.

This showcase covers the current camera presets and UI-emphasis combinations. Automatic cursor/subject tracking, whip transitions with directional motion blur, and multi-clip match cuts are outside the current renderer.
