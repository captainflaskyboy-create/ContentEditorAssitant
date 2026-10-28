//! Off-screen Direct2D compositor, using one shared D3D11 device, or WARP.
use crate::{
    error::{Error, Result},
    media::{self, Frame, VideoReader},
    model::*,
};
use serde_json::{json, Value};
use std::path::Path;
use windows::{
    core::{w, Interface},
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            Direct3D::*,
            Direct3D11::*,
            DirectWrite::*,
            Dxgi::{Common::*, *},
            Imaging::*,
        },
        Media::MediaFoundation::*,
        System::Com::*,
    },
};
use windows_numerics::Matrix3x2;

pub struct Graphics {
    pub device: ID3D11Device,
    pub context: ID2D1DeviceContext,
    pub manager: IMFDXGIDeviceManager,
    pub info: Value,
    pub hardware: bool,
}
impl Graphics {
    pub fn new(acceleration: Acceleration) -> Result<Self> {
        unsafe {
            let create = |driver| -> Result<ID3D11Device> {
                let mut d = None;
                let flags = if driver == D3D_DRIVER_TYPE_WARP {
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT
                } else {
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT
                };
                D3D11CreateDevice(
                    None,
                    driver,
                    HMODULE::default(),
                    flags,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut d),
                    None,
                    None,
                )?;
                Ok(d.unwrap())
            };
            let (device, hardware, fallback) = if acceleration == Acceleration::Auto {
                match create(D3D_DRIVER_TYPE_HARDWARE) {
                    Ok(d) => (d, true, None),
                    Err(e) => (create(D3D_DRIVER_TYPE_WARP)?, false, Some(e.to_string())),
                }
            } else {
                (create(D3D_DRIVER_TYPE_WARP)?, false, None)
            };
            let immediate = device.GetImmediateContext()?;
            if let Ok(multithread) = immediate.cast::<ID3D11Multithread>() {
                let _ = multithread.SetMultithreadProtected(true);
            }
            let dxgi: IDXGIDevice = device.cast()?;
            let adapter = dxgi.GetAdapter()?;
            let desc = adapter.GetDesc()?;
            let name = String::from_utf16_lossy(
                &desc.Description[..desc.Description.iter().position(|c| *c == 0).unwrap_or(128)],
            );
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d = factory.CreateDevice(&dxgi)?;
            let context = d2d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let mut token = 0;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.unwrap();
            manager.ResetDevice(&device, token)?;
            Ok(Self {
                device,
                context,
                manager,
                hardware,
                info: json!({"renderer":if hardware{"direct2d_d3d11"}else{"direct2d_warp"},"hardware":hardware,"adapter":name,"fallback_reason":fallback}),
            })
        }
    }
    pub fn bitmap(&self, w: u32, h: u32, flags: D2D1_BITMAP_OPTIONS) -> Result<ID2D1Bitmap1> {
        unsafe {
            Ok(self.context.CreateBitmap(
                D2D_SIZE_U {
                    width: w,
                    height: h,
                },
                None,
                0,
                &D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: flags,
                    colorContext: Default::default(),
                },
            )?)
        }
    }
    pub fn readback(&self, source: &ID2D1Bitmap1, cpu: &ID2D1Bitmap1) -> Result<Vec<u8>> {
        unsafe {
            cpu.CopyFromBitmap(None, source, None)?;
            let m = cpu.Map(D2D1_MAP_OPTIONS_READ)?;
            let size = source.GetPixelSize();
            let mut result = vec![0u8; (size.width * size.height * 4) as usize];
            for y in 0..size.height as usize {
                std::ptr::copy_nonoverlapping(
                    m.bits.add(y * m.pitch as usize),
                    result.as_mut_ptr().add(y * size.width as usize * 4),
                    size.width as usize * 4,
                );
            }
            cpu.Unmap()?;
            Ok(result)
        }
    }
    pub fn input(
        &self,
        reader: &VideoReader,
        frame: &Frame,
        rotation: i32,
        reused: &ID2D1Bitmap1,
    ) -> Result<(ID2D1Bitmap1, bool)> {
        unsafe {
            // Use decoder textures directly when the negotiated format is compatible with Direct2D.
            if rotation == 0 {
                if let Ok(buffer) = frame.sample.GetBufferByIndex(0) {
                    if let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() {
                        let mut raw = std::ptr::null_mut();
                        if dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw).is_ok()
                            && !raw.is_null()
                        {
                            let texture = ID3D11Texture2D::from_raw(raw);
                            let mut desc = D3D11_TEXTURE2D_DESC::default();
                            texture.GetDesc(&mut desc);
                            if desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM {
                                if let Ok(surface) = texture.cast::<IDXGISurface>() {
                                    if let Ok(bitmap) =
                                        self.context.CreateBitmapFromDxgiSurface(&surface, None)
                                    {
                                        return Ok((bitmap, true));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let mut data = reader.pixels(frame)?;
            if rotation.rem_euclid(360) != 0 {
                data = rotate(&data, reader.width, reader.height, rotation);
            }
            let size = reused.GetPixelSize();
            reused.CopyFromMemory(None, data.as_ptr().cast(), size.width * 4)?;
            Ok((reused.clone(), false))
        }
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
fn identity() -> Matrix3x2 {
    Matrix3x2 {
        M11: 1.0,
        M22: 1.0,
        ..Default::default()
    }
}
fn camera(req: &Request, t: f64) -> (f32, Point) {
    for e in req.effects.iter().filter(|e| e.camera()) {
        let (start, end) = e.interval();
        if t < start || t >= end {
            continue;
        }
        for keys in e.keyframes().windows(2) {
            let (a, b) = (keys[0], keys[1]);
            if t >= a.time && t < b.time {
                let u = ((t - a.time) / (b.time - a.time)).clamp(0.0, 1.0);
                let u = match a.easing {
                    Easing::Linear => u,
                    Easing::EaseIn => u * u,
                    Easing::EaseOut => 1.0 - (1.0 - u).powi(2),
                    Easing::EaseInOut => {
                        if u < 0.5 {
                            2.0 * u * u
                        } else {
                            1.0 - (-2.0 * u + 2.0).powi(2) / 2.0
                        }
                    }
                };
                return (
                    (a.zoom + (b.zoom - a.zoom) * u) as f32,
                    Point {
                        x: a.target.x + (b.target.x - a.target.x) * u,
                        y: a.target.y + (b.target.y - a.target.y) * u,
                    },
                );
            }
        }
    }
    (1.0, Point::default())
}
pub fn dimensions(req: &Request, m: &media::Media) -> (u32, u32) {
    let h = req.settings.preview_height.unwrap_or(m.height) / 2 * 2;
    let w = ((m.width as f64 * h as f64 / m.height as f64).round() as u32 / 2 * 2).max(2);
    (w, h)
}
pub fn bitrate(req: &Request, m: &media::Media) -> u32 {
    let (w, h) = dimensions(req, m);
    req.settings.video_bitrate_kbps.unwrap_or(
        (10_000.0 * (w as f64 * h as f64) / (1920.0 * 1080.0) * m.fps / 30.0)
            .round()
            .clamp(2000.0, 40000.0) as u32,
    )
}

pub struct Compositor<'a> {
    pub graphics: &'a Graphics,
    pub input: ID2D1Bitmap1,
    work: [ID2D1Bitmap1; 2],
    pub output: ID2D1Bitmap1,
    cpu: ID2D1Bitmap1,
    blur: ID2D1Effect,
    width: u32,
    height: u32,
    ow: u32,
    oh: u32,
}
impl<'a> Compositor<'a> {
    pub fn new(graphics: &'a Graphics, w: u32, h: u32, ow: u32, oh: u32) -> Result<Self> {
        unsafe {
            let input = graphics.bitmap(w, h, D2D1_BITMAP_OPTIONS_NONE)?;
            let work = [
                graphics.bitmap(w, h, D2D1_BITMAP_OPTIONS_TARGET)?,
                graphics.bitmap(w, h, D2D1_BITMAP_OPTIONS_TARGET)?,
            ];
            let output = graphics.bitmap(ow, oh, D2D1_BITMAP_OPTIONS_TARGET)?;
            let cpu = graphics.bitmap(
                ow,
                oh,
                D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            )?;
            let blur = graphics.context.CreateEffect(&CLSID_D2D1GaussianBlur)?;
            Ok(Self {
                graphics,
                input,
                work,
                output,
                cpu,
                blur,
                width: w,
                height: h,
                ow,
                oh,
            })
        }
    }
    pub fn draw(&self, input: &ID2D1Bitmap1, req: &Request, time: f64) -> Result<()> {
        unsafe {
            let c = &self.graphics.context;
            let full = rect(0.0, 0.0, self.width as f32, self.height as f32);
            let mut current = input.clone();
            let mut index = 0;
            for effect in req.effects.iter().filter(|e| !e.camera()) {
                let (start, end) = effect.interval();
                if time < start || time >= end {
                    continue;
                }
                let target = &self.work[index];
                index = 1 - index;
                c.SetTarget(target);
                c.SetTransform(&identity());
                c.BeginDraw();
                c.Clear(None);
                c.DrawBitmap(
                    &current,
                    Some(&full),
                    1.0,
                    D2D1_INTERPOLATION_MODE_LINEAR,
                    None,
                    None,
                );
                match effect {
                    Effect::Spotlight { rect: r, dim, .. } => {
                        let focus = rect(
                            (r.x * self.width as f64) as f32,
                            (r.y * self.height as f64) as f32,
                            (r.width * self.width as f64) as f32,
                            (r.height * self.height as f64) as f32,
                        );
                        let brush = c.CreateSolidColorBrush(
                            &D2D1_COLOR_F {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: *dim as f32,
                            },
                            None,
                        )?;
                        for area in [
                            rect(0.0, 0.0, full.right, focus.top),
                            rect(0.0, focus.bottom, full.right, full.bottom - focus.bottom),
                            rect(0.0, focus.top, focus.left, focus.bottom - focus.top),
                            rect(
                                focus.right,
                                focus.top,
                                full.right - focus.right,
                                focus.bottom - focus.top,
                            ),
                        ] {
                            c.FillRectangle(&area, &brush);
                        }
                    }
                    Effect::Blur { rect: r, sigma, .. } => {
                        let focus = rect(
                            (r.x * self.width as f64) as f32,
                            (r.y * self.height as f64) as f32,
                            (r.width * self.width as f64) as f32,
                            (r.height * self.height as f64) as f32,
                        );
                        self.blur.SetInput(0, &current, true);
                        self.blur.SetValue(
                            D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
                            D2D1_PROPERTY_TYPE_FLOAT,
                            &(*sigma as f32).to_ne_bytes(),
                        )?;
                        c.PushAxisAlignedClip(&focus, D2D1_ANTIALIAS_MODE_ALIASED);
                        c.DrawImage(
                            &self.blur.GetOutput()?,
                            None,
                            None,
                            D2D1_INTERPOLATION_MODE_LINEAR,
                            D2D1_COMPOSITE_MODE_SOURCE_OVER,
                        );
                        c.PopAxisAlignedClip();
                    }
                    _ => unreachable!(),
                }
                c.EndDraw(None, None)?;
                c.SetTarget(None);
                current = target.clone();
            }
            let (z, target) = camera(req, time);
            let cw = self.width as f32 / z;
            let ch = self.height as f32 / z;
            let x =
                (target.x as f32 * self.width as f32 - cw / 2.0).clamp(0.0, self.width as f32 - cw);
            let y = (target.y as f32 * self.height as f32 - ch / 2.0)
                .clamp(0.0, self.height as f32 - ch);
            c.SetTarget(&self.output);
            c.SetTransform(&identity());
            c.BeginDraw();
            c.Clear(None);
            c.DrawBitmap(
                &current,
                Some(&rect(0.0, 0.0, self.ow as f32, self.oh as f32)),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                Some(&rect(x, y, cw, ch)),
                None,
            );
            c.EndDraw(None, None)?;
            c.SetTarget(None);
            Ok(())
        }
    }
    pub fn pixels(&self) -> Result<Vec<u8>> {
        self.graphics.readback(&self.output, &self.cpu)
    }
}

pub fn rotate(src: &[u8], w: u32, h: u32, rotation: i32) -> Vec<u8> {
    let mut out = vec![0; src.len()];
    let (w, h) = (w as usize, h as usize);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy, dw) = match rotation.rem_euclid(360) {
                90 => (h - 1 - y, x, h),
                180 => (w - 1 - x, h - 1 - y, w),
                270 => (y, w - 1 - x, h),
                _ => (x, y, w),
            };
            out[(dy * dw + dx) * 4..(dy * dw + dx + 1) * 4]
                .copy_from_slice(&src[(y * w + x) * 4..(y * w + x + 1) * 4]);
        }
    }
    out
}

pub fn save_png(path: &Path, w: u32, h: u32, data: &[u8]) -> Result<()> {
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(&media::native_path(path), GENERIC_WRITE.0)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        let mut props = None;
        encoder.CreateNewFrame(&mut frame, &mut props)?;
        let frame = frame.unwrap();
        frame.Initialize(props.as_ref())?;
        frame.SetSize(w, h)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        if format != GUID_WICPixelFormat32bppBGRA {
            return Err(Error::new(
                "image_failed",
                "PNG encoder rejected BGRA pixels",
            ));
        }
        frame.WritePixels(h, w * 4, data)?;
        frame.Commit()?;
        encoder.Commit()?;
        Ok(())
    }
}

/// Draw contact-sheet labels using DirectWrite, keeping WIC as the PNG encoder.
pub fn label_sheet(
    g: &Graphics,
    bitmap: &ID2D1Bitmap1,
    labels: &[(String, D2D_RECT_F)],
) -> Result<()> {
    unsafe {
        let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let format = factory.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            16.0,
            w!("en-us"),
        )?;
        let brush = g.context.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            None,
        )?;
        g.context.SetTarget(bitmap);
        g.context.BeginDraw();
        for (text, area) in labels {
            g.context.DrawText(
                &text.encode_utf16().collect::<Vec<_>>(),
                &format,
                area,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        g.context.EndDraw(None, None)?;
        g.context.SetTarget(None);
        Ok(())
    }
}
