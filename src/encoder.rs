use crate::{
    error::{Error, Result},
    graphics::{self, Graphics},
    media::{self, *},
    model::{Acceleration, Request},
};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};
use windows::{
    core::{implement, Interface, Ref, GUID},
    Win32::{
        Foundation::E_NOTIMPL,
        Graphics::{
            Direct3D11::*,
            Dxgi::{Common::*, IDXGISurface2},
        },
        Media::MediaFoundation::*,
    },
};

fn video_type(subtype: &GUID, w: u32, h: u32, rate: (u32, u32)) -> Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, packed(w, h))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, packed(rate.0, rate.1))?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, packed(1, 1))?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        Ok(t)
    }
}
pub struct Encoder {
    pub writer: IMFSinkWriter,
    pub video: u32,
    pub audio: Option<u32>,
    pub info: Value,
    pub gpu_input: bool,
    finalized: bool,
}
impl Encoder {
    pub fn new(
        path: &Path,
        req: &Request,
        source: &Media,
        g: &Graphics,
        allow_gpu: bool,
    ) -> Result<Self> {
        let (w, h) = graphics::dimensions(req, source);
        let mut rate = source
            .frame_rate
            .split('/')
            .map(|s| s.parse::<u32>().unwrap_or(1));
        let rate = (rate.next().unwrap_or(30), rate.next().unwrap_or(1));
        let setup = |hardware: bool, manager: bool| -> Result<Self> {
            unsafe {
                let a = attributes()?;
                a.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, hardware as u32)?;
                a.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
                a.SetUINT32(&MF_LOW_LATENCY, 1)?;
                if manager {
                    a.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &g.manager)?;
                }
                let writer = MFCreateSinkWriterFromURL(&media::native_path(path), None, Some(&a))?;
                let output = video_type(&MFVideoFormat_H264, w, h, rate)?;
                output.SetUINT32(&MF_MT_AVG_BITRATE, graphics::bitrate(req, source) * 1000)?;
                let video = writer.AddStream(&output)?;
                let input = video_type(
                    if manager {
                        &MFVideoFormat_ARGB32
                    } else {
                        &MFVideoFormat_RGB32
                    },
                    w,
                    h,
                    rate,
                )?;
                input.SetUINT32(&MF_MT_DEFAULT_STRIDE, w * 4)?;
                writer.SetInputMediaType(video, &input, None)?;
                let audio = if source.has_audio {
                    let output = audio_type(&MFAudioFormat_AAC)?;
                    output.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 24000)?;
                    output.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
                    let index = writer.AddStream(&output)?;
                    let input = audio_type(&MFAudioFormat_PCM)?;
                    writer.SetInputMediaType(index, &input, None)?;
                    Some(index)
                } else {
                    None
                };
                writer.BeginWriting()?;
                let mut transforms = vec![];
                if let Ok(ex) = writer.cast::<IMFSinkWriterEx>() {
                    for i in 0..8 {
                        let mut transform = None;
                        let mut category = GUID::zeroed();
                        if ex
                            .GetTransformForStream(video, i, Some(&mut category), &mut transform)
                            .is_err()
                        {
                            break;
                        }
                        if let Some(t) = transform {
                            let mut v = transform_info(&t);
                            v["category"] = json!(format!("{category:?}"));
                            transforms.push(v);
                        }
                    }
                }
                let actual_hardware = transforms.iter().any(|t| t["hardware"] == true);
                Ok(Self {
                    writer,
                    video,
                    audio,
                    gpu_input: manager,
                    finalized: false,
                    info: json!({"transforms":transforms,"hardware_observed":actual_hardware,"hardware_requested":hardware,"input":if manager{"d3d11_bgra_surface"}else{"system_memory_bgra"},"fallback_reason":Value::Null}),
                })
            }
        };
        if req.settings.acceleration == Acceleration::Auto {
            match setup(true, g.hardware && allow_gpu) {
                Ok(s) => Ok(s),
                Err(first) => {
                    let mut fallback=setup(false,false).map_err(|e|Error::new("encoder_unavailable",format!("Windows H.264/AAC encoder setup failed. Hardware attempt: {first}; software attempt: {e}")))?;
                    fallback.info["fallback_reason"] = json!(first.to_string());
                    Ok(fallback)
                }
            }
        } else {
            setup(false, false)
        }
    }
    pub fn video(&self, pixels: &[u8], time: i64, duration: i64) -> Result<()> {
        let s = sample_bytes(pixels, time, duration)?;
        unsafe {
            self.writer.WriteSample(self.video, &s)?;
        }
        Ok(())
    }
    pub fn finish(mut self) -> Result<()> {
        unsafe {
            self.writer.Finalize().map_err(|e| {
                Error::new("encoder_failed", format!("Could not finalize MP4: {e}"))
            })?;
        }
        self.finalized = true;
        Ok(())
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        if !self.finalized {
            unsafe {
                let _ = self.writer.Flush(self.video);
                if let Some(a) = self.audio {
                    let _ = self.writer.Flush(a);
                }
                let _ = self.writer.Finalize();
            }
        }
    }
}

#[implement(IMFAsyncCallback)]
struct Recycle {
    slot: usize,
    sender: std::sync::mpsc::Sender<usize>,
}
impl IMFAsyncCallback_Impl for Recycle_Impl {
    fn GetParameters(&self, _flags: *mut u32, _queue: *mut u32) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Invoke(&self, _result: Ref<IMFAsyncResult>) -> windows::core::Result<()> {
        let _ = self.sender.send(self.slot);
        Ok(())
    }
}
/// A bounded GPU pool. Tracked samples recycle a texture only after downstream
/// transforms release it, preventing overwrites while an async encoder reads it.
pub struct SurfacePool {
    textures: Vec<ID3D11Texture2D>,
    callbacks: Vec<IMFAsyncCallback>,
    available: std::sync::mpsc::Receiver<usize>,
}
impl SurfacePool {
    pub fn new(g: &Graphics, w: u32, h: u32) -> Result<Self> {
        unsafe {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut textures = vec![];
            let mut callbacks = vec![];
            for i in 0..8 {
                let mut texture = None;
                g.device.CreateTexture2D(
                    &D3D11_TEXTURE2D_DESC {
                        Width: w,
                        Height: h,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        Usage: D3D11_USAGE_DEFAULT,
                        BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
                        CPUAccessFlags: 0,
                        MiscFlags: 0,
                    },
                    None,
                    Some(&mut texture),
                )?;
                textures.push(texture.unwrap());
                callbacks.push(
                    Recycle {
                        slot: i,
                        sender: tx.clone(),
                    }
                    .into(),
                );
                tx.send(i).unwrap();
            }
            Ok(Self {
                textures,
                callbacks,
                available: rx,
            })
        }
    }
    pub fn write(
        &self,
        g: &Graphics,
        encoder: &Encoder,
        bitmap: &windows::Win32::Graphics::Direct2D::ID2D1Bitmap1,
        time: i64,
        duration: i64,
        cancel: &Arc<AtomicBool>,
    ) -> Result<()> {
        unsafe {
            let started = std::time::Instant::now();
            let index = loop {
                media::check_cancel(cancel)?;
                match self
                    .available
                    .recv_timeout(std::time::Duration::from_millis(30))
                {
                    Ok(i) => break i,
                    Err(_) => {
                        if started.elapsed().as_secs() > 5 {
                            return Err(Error::new(
                                "gpu_surface_failed",
                                "Native encoder did not release a GPU sample within five seconds",
                            ));
                        }
                    }
                }
            };
            let surface: IDXGISurface2 = bitmap.GetSurface()?.cast()?;
            let mut sub = 0;
            let source: ID3D11Texture2D = surface.GetResource(&mut sub)?;
            g.device
                .GetImmediateContext()?
                .CopyResource(&self.textures[index], &source);
            let buffer =
                MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &self.textures[index], 0, false)?;
            buffer.SetCurrentLength(buffer.GetMaxLength()?)?;
            let tracked = MFCreateTrackedSample()?;
            let sample: IMFSample = tracked.cast()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(time)?;
            sample.SetSampleDuration(duration)?;
            tracked.SetAllocator(&self.callbacks[index], None)?;
            encoder
                .writer
                .WriteSample(encoder.video, &sample)
                .map_err(|e| {
                    Error::new(
                        "gpu_surface_failed",
                        format!("GPU surface encoding failed: {e}"),
                    )
                })?;
            Ok(())
        }
    }
}
pub fn audio_type(subtype: &GUID) -> Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48000)?;
        if *subtype == MFAudioFormat_PCM {
            t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            t.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
            t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 192000)?;
            t.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        }
        Ok(t)
    }
}

/// Pull audio in bounded chunks and pad gaps with silence, on the source clock.
pub struct Audio {
    reader: IMFSourceReader,
    start: i64,
    next: i64,
    chunk: Vec<u8>,
    chunk_start: i64,
    eof: bool,
    offset: i64,
    expected_chunk_start: Option<i64>,
}
impl Audio {
    pub fn new(source: &Path, start: f64) -> Result<Self> {
        unsafe {
            let r = media::open(source, None)?;
            r.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            r.SetStreamSelection(FIRST_AUDIO, true)?;
            r.SetCurrentMediaType(FIRST_AUDIO, None, &audio_type(&MFAudioFormat_PCM)?)?;
            let delay = media::ticks(crate::mp4::audio_delay(source));
            let mut offset = 0;
            if delay > 0 {
                let mut first = None;
                let mut flags = 0;
                let mut time = 0;
                for _ in 0..32 {
                    r.ReadSample(
                        FIRST_AUDIO,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut time),
                        Some(&mut first),
                    )?;
                    if first.is_some() || flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                        break;
                    }
                }
                if first.is_some() && time.abs() < 100_000 {
                    offset = delay;
                }
            }
            r.SetCurrentPosition(
                &GUID::zeroed(),
                &windows::Win32::System::Com::StructuredStorage::PROPVARIANT::from(
                    (ticks(start) - offset).max(0),
                ),
            )?;
            Ok(Self {
                reader: r,
                start: ticks(start),
                next: 0,
                chunk: vec![],
                chunk_start: 0,
                eof: false,
                offset,
                expected_chunk_start: None,
            })
        }
    }
    fn read(&mut self) -> Result<()> {
        unsafe {
            loop {
                let mut flags = 0;
                let mut time = 0;
                let mut sample = None;
                self.reader.ReadSample(
                    FIRST_AUDIO,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut time),
                    Some(&mut sample),
                )?;
                if flags & MF_SOURCE_READERF_STREAMTICK.0 as u32 != 0 {
                    // Explicit source gaps must retain their presentation timing.
                    self.expected_chunk_start = None;
                }
                if let Some(s) = sample {
                    let b = s.ConvertToContiguousBuffer()?;
                    let mut ptr = std::ptr::null_mut();
                    let mut len = 0;
                    b.Lock(&mut ptr, None, Some(&mut len))?;
                    self.chunk = std::slice::from_raw_parts(ptr, len as usize).to_vec();
                    b.Unlock()?;
                    let reported_start =
                        ((time + self.offset - self.start) as f64 / TICKS * 48000.0).round() as i64;
                    // MKV timecodes can be rounded to milliseconds, although an AAC
                    // packet still decodes to exactly 1024 samples. Follow the PCM
                    // sample clock for <=2ms timestamp jitter instead of inserting
                    // zeros or discarding samples at virtually every packet boundary.
                    self.chunk_start =
                        continuous_audio_start(self.expected_chunk_start, reported_start);
                    self.expected_chunk_start =
                        Some(self.chunk_start + (self.chunk.len() / 4) as i64);
                    return Ok(());
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    self.eof = true;
                    self.chunk.clear();
                    return Ok(());
                }
                if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                    return Err(Error::new("decode_failed", "Audio decoder failed"));
                }
            }
        }
    }
    pub fn write_until(
        &mut self,
        encoder: &Encoder,
        end: i64,
        cancel: &Arc<AtomicBool>,
    ) -> Result<()> {
        while self.next < end {
            check_cancel(cancel)?;
            let stop = (self.next + 1024).min(end);
            let mut bytes = vec![0u8; ((stop - self.next) * 4) as usize];
            let mut pos = self.next;
            while pos < stop {
                if self.chunk.is_empty() && !self.eof {
                    self.read()?;
                }
                if self.eof {
                    break;
                }
                let chunk_end = self.chunk_start + (self.chunk.len() / 4) as i64;
                if chunk_end <= pos {
                    self.chunk.clear();
                    continue;
                }
                if self.chunk_start >= stop {
                    break;
                }
                let first = pos.max(self.chunk_start);
                let last = stop.min(chunk_end);
                if last > first {
                    bytes[((first - self.next) * 4) as usize..((last - self.next) * 4) as usize]
                        .copy_from_slice(
                            &self.chunk[((first - self.chunk_start) * 4) as usize
                                ..((last - self.chunk_start) * 4) as usize],
                        );
                    pos = last;
                }
                if pos >= chunk_end {
                    self.chunk.clear();
                }
            }
            let time = (self.next as i128 * 10_000_000 / 48000) as i64;
            let finish = (stop as i128 * 10_000_000 / 48000) as i64;
            let s = sample_bytes(&bytes, time, finish - time)?;
            unsafe {
                encoder.writer.WriteSample(encoder.audio.unwrap(), &s)?;
            }
            self.next = stop;
        }
        Ok(())
    }
}

fn continuous_audio_start(expected: Option<i64>, reported: i64) -> i64 {
    expected
        .filter(|next| next.abs_diff(reported) <= 96)
        .unwrap_or(reported)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_clock_absorbs_quantization_but_preserves_real_gaps() {
        assert_eq!(continuous_audio_start(Some(1024), 1008), 1024);
        assert_eq!(continuous_audio_start(Some(2048), 2064), 2048);
        assert_eq!(continuous_audio_start(Some(1024), 5824), 5824);
        assert_eq!(continuous_audio_start(None, 24000), 24000);
    }
    #[test]
    fn invalid_encoder_sample_closes_and_discards_staging() {
        let _runtime = media::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let output = d.path().join("final.mp4");
        std::fs::write(&output, b"previous-final").unwrap();
        let req:Request=serde_json::from_value(json!({"version":2,"source":"unused.mp4","output":output,"range":{"start":0,"duration":1},"settings":{"acceleration":"software"},"overwrite":true})).unwrap();
        let source = Media {
            source: "unused.mp4".into(),
            width: 320,
            height: 180,
            duration: 1.0,
            frame_rate: "30/1".into(),
            fps: 30.0,
            has_audio: false,
            audio_streams: vec![],
            sample_aspect_ratio: "1:1".into(),
            rotation: 0,
        };
        let g = Graphics::new(Acceleration::Software).unwrap();
        let stage = tempfile::Builder::new()
            .suffix(".partial.mp4")
            .tempfile_in(d.path())
            .unwrap()
            .into_temp_path();
        let name = stage.to_path_buf();
        {
            let encoder = Encoder::new(&stage, &req, &source, &g, false).unwrap();
            assert!(encoder.video(&[], 0, 333333).is_err());
        }
        stage.close().unwrap();
        assert!(!name.exists());
        assert_eq!(std::fs::read(&output).unwrap(), b"previous-final");
    }
}
