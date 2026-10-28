//! Native Media Foundation I/O. No process spawning or external codec tools.
use crate::{
    error::{Error, Result},
    model::Acceleration,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use windows::{
    core::{Interface, GUID, HSTRING},
    Win32::{
        Media::MediaFoundation::*,
        System::{Com::StructuredStorage::*, Com::*},
    },
};

pub const TICKS: f64 = 10_000_000.0;
pub const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
pub const FIRST_AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
pub struct Runtime;
impl Runtime {
    pub fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(Error::new("media_unavailable",format!("Windows Media Foundation is unavailable. Windows N editions need the Media Feature Pack. {e}")));
            }
        }
        Ok(Self)
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}
pub fn attributes() -> Result<IMFAttributes> {
    let mut a = None;
    unsafe {
        MFCreateAttributes(&mut a, 8)?;
    }
    Ok(a.unwrap())
}
pub fn packed(a: u32, b: u32) -> u64 {
    ((a as u64) << 32) | b as u64
}
pub fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}
pub fn ticks(t: f64) -> i64 {
    (t * TICKS).round() as i64
}
pub fn check_cancel(cancel: &Arc<AtomicBool>) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(Error::new(
            "cancelled",
            "Operation cancelled; current partial output discarded",
        ))
    } else {
        Ok(())
    }
}
pub fn event(v: Value) {
    eprintln!("{v}");
}
pub fn local_source(path: &Path) -> Result<PathBuf> {
    if !path.is_file() {
        return Err(Error::new(
            "invalid_source",
            format!("Not a local file: {}", path.display()),
        ));
    }
    Ok(path.canonicalize()?)
}
pub fn native_path(source: &Path) -> HSTRING {
    let s = source.to_string_lossy();
    HSTRING::from(s.strip_prefix("\\\\?\\").unwrap_or(&s))
}
pub fn open(source: &Path, attrs: Option<&IMFAttributes>) -> Result<IMFSourceReader> {
    unsafe {
        MFCreateSourceReaderFromURL(&native_path(source),attrs).map_err(|e|Error::new("unsupported_media",format!("Windows could not open {}. Container/codec support depends on installed Windows media components. {e}",source.display())))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Media {
    pub source: PathBuf,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub frame_rate: String,
    pub fps: f64,
    pub has_audio: bool,
    pub audio_streams: Vec<Value>,
    pub sample_aspect_ratio: String,
    pub rotation: i32,
}
pub fn inspect(source: &Path) -> Result<Media> {
    let source = local_source(source)?;
    let r = open(&source, None)?;
    unsafe {
        let v = r.GetNativeMediaType(FIRST_VIDEO, 0).map_err(|e| {
            Error::new(
                "invalid_media",
                format!("A decodable video stream is required: {e}"),
            )
        })?;
        let (mut width, mut height) = unpack(v.GetUINT64(&MF_MT_FRAME_SIZE)?);
        let (mut num, mut den) = unpack(v.GetUINT64(&MF_MT_FRAME_RATE)?);
        if den == 0 || num == 0 {
            return Err(Error::new(
                "invalid_media",
                "Video has no usable frame rate",
            ));
        }
        for (n, d) in [
            (24, 1),
            (25, 1),
            (30, 1),
            (50, 1),
            (60, 1),
            (120, 1),
            (24000, 1001),
            (30000, 1001),
            (60000, 1001),
        ] {
            let rate = n as f64 / d as f64;
            if (num as f64 / den as f64 - rate).abs() < rate * 0.00001 {
                num = n;
                den = d;
                break;
            }
        }
        let duration = PropVariantToInt64(
            &r.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)?,
        )? as f64
            / TICKS;
        if !duration.is_finite() || duration <= 0.0 {
            return Err(Error::new(
                "invalid_media",
                "Video must have known positive duration",
            ));
        }
        let rotation = shell_rotation(&source)
            .unwrap_or_else(|| v.GetUINT32(&MF_MT_VIDEO_ROTATION).unwrap_or(0) as i32);
        if rotation.rem_euclid(180) == 90 {
            std::mem::swap(&mut width, &mut height);
        }
        let (sarx, sary) = unpack(
            v.GetUINT64(&MF_MT_PIXEL_ASPECT_RATIO)
                .unwrap_or(packed(1, 1)),
        );
        let mut audio_streams = vec![];
        for index in 0..64 {
            match r.GetNativeMediaType(index, 0) {
                Ok(t) => {
                    if t.GetGUID(&MF_MT_MAJOR_TYPE).ok() == Some(MFMediaType_Audio) {
                        audio_streams.push(json!({"index":index,"sample_rate":t.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).ok(),"channels":t.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).ok(),"subtype":format!("{:?}",t.GetGUID(&MF_MT_SUBTYPE)?)}));
                    }
                }
                Err(_) => break,
            }
        }
        Ok(Media {
            source,
            width,
            height,
            duration,
            frame_rate: format!("{num}/{den}"),
            fps: num as f64 / den as f64,
            has_audio: !audio_streams.is_empty(),
            audio_streams,
            sample_aspect_ratio: format!("{sarx}:{sary}"),
            rotation,
        })
    }
}

fn shell_rotation(source: &Path) -> Option<i32> {
    unsafe {
        use windows::Win32::{Foundation::PROPERTYKEY, UI::Shell::PropertiesSystem::*};
        let store: IPropertyStore =
            SHGetPropertyStoreFromParsingName(&native_path(source), None, GPS_DEFAULT).ok()?;
        let mut key = PROPERTYKEY::default();
        PSGetPropertyKeyFromName(windows::core::w!("System.Video.Orientation"), &mut key).ok()?;
        let value = store.GetValue(&key).ok()?;
        PropVariantToUInt32(&value).ok().map(|v| v as i32)
    }
}

/// Observe transform attributes, rather than inferring acceleration from GPU presence.
pub fn transform_info(transform: &IMFTransform) -> Value {
    unsafe {
        let Ok(a) = transform.GetAttributes() else {
            return json!({"hardware":"unknown"});
        };
        let hardware = a.GetStringLength(&MFT_ENUM_HARDWARE_URL_Attribute).is_ok();
        let mut name = vec![0u16; 512];
        let _ = a.GetString(&MFT_FRIENDLY_NAME_Attribute, &mut name, None);
        let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
        json!({"hardware":hardware,"name":String::from_utf16_lossy(&name[..end])})
    }
}
pub fn reader_transforms(reader: &IMFSourceReader) -> Vec<Value> {
    let mut result = vec![];
    if let Ok(ex) = reader.cast::<IMFSourceReaderEx>() {
        unsafe {
            for i in 0..8 {
                let mut t = None;
                let mut category = GUID::zeroed();
                if ex
                    .GetTransformForStream(FIRST_VIDEO, i, Some(&mut category), &mut t)
                    .is_err()
                {
                    break;
                }
                if let Some(t) = t {
                    let mut v = transform_info(&t);
                    v["category"] = json!(format!("{category:?}"));
                    result.push(v);
                }
            }
        }
    }
    result
}

pub struct VideoReader {
    pub reader: IMFSourceReader,
    pub width: u32,
    pub height: u32,
    pub stride: i32,
    pub transforms: Vec<Value>,
    pub warnings: Vec<String>,
}
pub struct Frame {
    pub sample: IMFSample,
    pub time: i64,
}
impl VideoReader {
    pub fn new(
        source: &Path,
        acceleration: Acceleration,
        manager: Option<&IMFDXGIDeviceManager>,
    ) -> Result<Self> {
        unsafe {
            let attrs = attributes()?;
            attrs.SetUINT32(
                &MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
                (acceleration == Acceleration::Auto) as u32,
            )?;
            attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
            if let Some(manager) = manager {
                attrs.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, manager)?;
            }
            let mut warnings = vec![];
            let configure = |a: &IMFAttributes| -> Result<IMFSourceReader> {
                let r = open(source, Some(a))?;
                r.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
                r.SetStreamSelection(FIRST_VIDEO, true)?;
                let ty = MFCreateMediaType()?;
                ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                ty.SetGUID(
                    &MF_MT_SUBTYPE,
                    if manager.is_some() {
                        &MFVideoFormat_ARGB32
                    } else {
                        &MFVideoFormat_RGB32
                    },
                )?;
                r.SetCurrentMediaType(FIRST_VIDEO, None, &ty)?;
                Ok(r)
            };
            let reader = match configure(&attrs) {
                Ok(r) => r,
                Err(e) if manager.is_some() => {
                    warnings.push(format!("GPU decoder surface negotiation failed; native system-memory decoding used: {e}"));
                    attrs.DeleteItem(&MF_SOURCE_READER_D3D_MANAGER)?;
                    configure(&attrs)?
                }
                Err(e) => return Err(e),
            };
            let ty = reader.GetCurrentMediaType(FIRST_VIDEO)?;
            let (width, height) = unpack(ty.GetUINT64(&MF_MT_FRAME_SIZE)?);
            let stride = ty.GetUINT32(&MF_MT_DEFAULT_STRIDE).unwrap_or(width * 4) as i32;
            let transforms = reader_transforms(&reader);
            Ok(Self {
                reader,
                width,
                height,
                stride,
                transforms,
                warnings,
            })
        }
    }
    pub fn seek(&self, t: f64) -> Result<()> {
        unsafe {
            self.reader
                .SetCurrentPosition(&GUID::zeroed(), &PROPVARIANT::from(ticks(t)))?;
        }
        Ok(())
    }
    pub fn next(&self) -> Result<Option<Frame>> {
        unsafe {
            loop {
                let mut flags = 0;
                let mut time = 0;
                let mut sample = None;
                self.reader.ReadSample(
                    FIRST_VIDEO,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut time),
                    Some(&mut sample),
                )?;
                if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                    return Err(Error::new(
                        "decode_failed",
                        "Windows video decoder reported an error",
                    ));
                }
                if flags & (MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32) != 0 {
                    let t = self.reader.GetCurrentMediaType(FIRST_VIDEO)?;
                    if unpack(t.GetUINT64(&MF_MT_FRAME_SIZE)?) != (self.width, self.height) {
                        return Err(Error::new(
                            "unsupported_media",
                            "Mid-stream video dimension changes are not supported",
                        ));
                    }
                }
                if let Some(sample) = sample {
                    return Ok(Some(Frame { sample, time }));
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    return Ok(None);
                }
            }
        }
    }
    pub fn at(&self, time: f64, cancel: &Arc<AtomicBool>) -> Result<Frame> {
        self.seek(time)?;
        loop {
            check_cancel(cancel)?;
            let f = self.next()?.ok_or_else(|| {
                Error::new("frame_unavailable", format!("No frame at {time} seconds"))
            })?;
            if f.time >= ticks(time) {
                return Ok(f);
            }
        }
    }
    pub fn pixels(&self, frame: &Frame) -> Result<Vec<u8>> {
        unsafe {
            let b = frame.sample.ConvertToContiguousBuffer()?;
            let mut data = vec![0u8; (self.width * self.height * 4) as usize];
            if let Ok(two) = b.cast::<IMF2DBuffer>() {
                let mut ptr = std::ptr::null_mut();
                let mut pitch = 0;
                two.Lock2D(&mut ptr, &mut pitch)?;
                for y in 0..self.height as usize {
                    std::ptr::copy_nonoverlapping(
                        ptr.offset(y as isize * pitch as isize),
                        data.as_mut_ptr().add(y * self.width as usize * 4),
                        self.width as usize * 4,
                    );
                }
                two.Unlock2D()?;
            } else {
                let mut ptr = std::ptr::null_mut();
                let mut len = 0;
                b.Lock(&mut ptr, None, Some(&mut len))?;
                let stride = self.stride.unsigned_abs() as usize;
                let needed = stride * (self.height as usize - 1) + self.width as usize * 4;
                if (len as usize) < needed {
                    b.Unlock()?;
                    return Err(Error::new(
                        "decode_failed",
                        "Decoded frame buffer is too small",
                    ));
                }
                for y in 0..self.height as usize {
                    let sy = if self.stride < 0 {
                        self.height as usize - 1 - y
                    } else {
                        y
                    };
                    std::ptr::copy_nonoverlapping(
                        ptr.add(sy * stride),
                        data.as_mut_ptr().add(y * self.width as usize * 4),
                        self.width as usize * 4,
                    );
                }
                b.Unlock()?;
            }
            for p in data.as_chunks_mut::<4>().0 {
                p[3] = 255;
            }
            Ok(data)
        }
    }
}

pub fn sample_bytes(bytes: &[u8], time: i64, duration: i64) -> Result<IMFSample> {
    unsafe {
        let b = MFCreateMemoryBuffer(bytes.len() as u32)?;
        let mut ptr = std::ptr::null_mut();
        b.Lock(&mut ptr, None, None)?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        b.Unlock()?;
        b.SetCurrentLength(bytes.len() as u32)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&b)?;
        sample.SetSampleTime(time)?;
        sample.SetSampleDuration(duration)?;
        Ok(sample)
    }
}

pub fn encoders(category: GUID, subtype: GUID) -> Result<Vec<Value>> {
    unsafe {
        let mut ptr = std::ptr::null_mut();
        let mut count = 0;
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: if category == MFT_CATEGORY_AUDIO_ENCODER {
                MFMediaType_Audio
            } else {
                MFMediaType_Video
            },
            guidSubtype: subtype,
        };
        MFTEnumEx(
            category,
            MFT_ENUM_FLAG_ALL,
            None,
            Some(&output),
            &mut ptr,
            &mut count,
        )?;
        let mut result = vec![];
        if !ptr.is_null() {
            for entry in std::slice::from_raw_parts_mut(ptr, count as usize) {
                if let Some(a) = entry.take() {
                    let mut name = vec![0u16; 512];
                    let _ = a.GetString(&MFT_FRIENDLY_NAME_Attribute, &mut name, None);
                    let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
                    result.push(json!({"name":String::from_utf16_lossy(&name[..end]),"hardware_registered":a.GetStringLength(&MFT_ENUM_HARDWARE_URL_Attribute).is_ok()}));
                }
            }
            CoTaskMemFree(Some(ptr.cast()));
        }
        Ok(result)
    }
}
pub fn doctor() -> Result<Value> {
    let g = crate::graphics::Graphics::new(Acceleration::Auto)?;
    let warp = crate::graphics::Graphics::new(Acceleration::Software)?;
    let h264 = encoders(MFT_CATEGORY_VIDEO_ENCODER, MFVideoFormat_H264)?;
    let aac = encoders(MFT_CATEGORY_AUDIO_ENCODER, MFAudioFormat_AAC)?;
    Ok(
        json!({"ok":!h264.is_empty()&&!aac.is_empty(),"backend":"windows_native","external_dependencies":[],"graphics":g.info,"software_graphics":warp.info,"h264_encoders":h264,"aac_encoders":aac,"note":"Registered encoders are candidates. Each render reports observed transforms and surface transfers; registration alone does not prove active acceleration."}),
    )
}
