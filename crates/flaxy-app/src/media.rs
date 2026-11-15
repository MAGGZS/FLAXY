//! Imagem e vídeo na interface sem navegador: o FFmpeg decodifica os quadros
//! direto para RGBA e eles viram textura na GPU.
//!
//! - `Thumbs`: miniaturas em cache (um quadro por arquivo/tempo/tamanho).
//! - `Player`: prévia com reprodução, pausa e busca. Mudo por padrão (é para
//!   enquadrar e cortar); `with_audio` liga o som com volume.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureOptions};

use crate::backend::{Backend, Task};

fn ffmpeg() -> Option<Command> {
    let p = flaxy_core::media::ffmpeg()?;
    Some(no_window(Command::new(p)))
}

fn no_window(mut c: Command) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Meta {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
}

/// Largura, altura e duração pelo ffprobe (bloqueante; chamar fora da UI).
pub fn probe_sync(path: &Path) -> Option<Meta> {
    let fp = flaxy_core::media::ffprobe()?;
    let out = no_window(Command::new(fp))
        .args([
            "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height:format=duration",
            "-of", "json",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let s = &v["streams"][0];
    Some(Meta {
        width: s["width"].as_u64()? as u32,
        height: s["height"].as_u64()? as u32,
        duration: v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0),
    })
}

/// Um quadro como PNG pelo pipe (tamanho final decidido pelo FFmpeg).
fn grab_png(path: &Path, at: f64, max_w: u32, max_h: u32) -> Option<ColorImage> {
    let run = |t: f64| -> Option<Vec<u8>> {
        let mut c = ffmpeg()?;
        c.args(["-hide_banner", "-loglevel", "error"]);
        if t > 0.0 {
            c.args(["-ss", &format!("{t:.3}")]);
        }
        let out = c
            .arg("-i")
            .arg(path)
            .args([
                "-frames:v", "1", "-vf",
                &format!("scale=w={max_w}:h={max_h}:force_original_aspect_ratio=decrease:flags=bilinear"),
                "-f", "image2pipe", "-c:v", "png", "-",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        (!out.stdout.is_empty()).then_some(out.stdout)
    };
    // Tempo além do fim não devolve nada: tenta o começo.
    let bytes = run(at).or_else(|| if at > 0.0 { run(0.0) } else { None })?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

enum Thumb {
    Loading(Task<Option<ColorImage>>),
    Ready(TextureHandle),
    Failed,
}

/// Cache de miniaturas. Até 3 FFmpeg simultâneos (ver `Backend::thumb_slots`).
#[derive(Default)]
pub struct Thumbs {
    map: HashMap<String, Thumb>,
}

impl Thumbs {
    pub fn get(&mut self, bk: &Arc<Backend>, ctx: &egui::Context, path: &Path, at: f64, max: (u32, u32)) -> Option<TextureHandle> {
        let key = format!("{}|{at:.2}|{}x{}", path.display(), max.0, max.1);
        if self.map.len() > 600 {
            self.map.retain(|_, t| matches!(t, Thumb::Loading(_)));
        }
        let entry = self.map.entry(key.clone()).or_insert_with(|| {
            let (p, slots) = (path.to_path_buf(), bk.thumb_slots.clone());
            Thumb::Loading(bk.spawn(ctx, async move {
                let _permit = slots.acquire_owned().await.ok()?;
                tokio::task::spawn_blocking(move || grab_png(&p, at, max.0, max.1)).await.ok().flatten()
            }))
        });
        if let Thumb::Loading(t) = entry {
            if let Some(r) = t.take() {
                *entry = match r {
                    Some(img) => Thumb::Ready(ctx.load_texture(key, img, TextureOptions::LINEAR)),
                    None => Thumb::Failed,
                };
            }
        }
        match entry {
            Thumb::Ready(t) => Some(t.clone()),
            _ => None,
        }
    }

    /// Esquece as miniaturas de um arquivo (ex.: fundo trocado).
    pub fn forget(&mut self, path: &Path) {
        let prefix = format!("{}|", path.display());
        self.map.retain(|k, _| !k.starts_with(&prefix));
    }
}

/// Desenha a textura cobrindo (`cover`) ou cabendo (`contain`) no retângulo.
pub fn paint_image(ui: &egui::Ui, tex: &TextureHandle, rect: Rect, cover: bool) {
    let [w, h] = tex.size();
    let (iw, ih) = (w as f32, h as f32);
    let painter = ui.painter_at(rect);
    let s = if cover { (rect.width() / iw).max(rect.height() / ih) } else { (rect.width() / iw).min(rect.height() / ih) };
    let r = Rect::from_center_size(rect.center(), egui::vec2(iw * s, ih * s));
    painter.image(tex.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
}

/// Retângulo com a proporção `aspect` centralizado dentro de `outer`.
pub fn fit_rect(outer: Rect, aspect: f32) -> Rect {
    let a = if aspect.is_finite() && aspect > 0.0 { aspect } else { 16.0 / 9.0 };
    let (w, h) = if outer.width() / outer.height() > a {
        (outer.height() * a, outer.height())
    } else {
        (outer.width(), outer.width() / a)
    };
    Rect::from_center_size(outer.center(), egui::vec2(w, h))
}

// ------------------------------------------------------------------ player

struct Frame {
    t: f64,
    rgba: Vec<u8>,
}

struct Session {
    rx: Receiver<Frame>,
    child: Arc<Mutex<Option<Child>>>,
    from: f64,
    started: Option<Instant>,
    pending: Option<Frame>,
    audio: Option<Audio>,
}

const RATE: u32 = 48_000;

/// Som do trecho: um FFmpeg decodifica para f32 estéreo e o rodio toca.
struct Audio {
    _out: rodio::OutputStream,
    sink: rodio::Sink,
    child: Arc<Mutex<Option<Child>>>,
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.sink.stop();
        kill(&self.child);
    }
}

fn kill(child: &Arc<Mutex<Option<Child>>>) {
    if let Ok(mut c) = child.lock() {
        if let Some(mut ch) = c.take() {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }
}

/// Amostras vindas do FFmpeg. Sem dados ainda, toca silêncio em quadros
/// estéreo inteiros para não trocar os canais.
struct Pcm {
    rx: Receiver<Vec<f32>>,
    buf: Vec<f32>,
    i: usize,
}

impl Iterator for Pcm {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.i >= self.buf.len() {
            self.buf = match self.rx.try_recv() {
                Ok(b) => b,
                Err(TryRecvError::Empty) => vec![0.0; 64],
                Err(TryRecvError::Disconnected) => return None,
            };
            self.i = 0;
        }
        let v = self.buf.get(self.i).copied();
        self.i += 1;
        v
    }
}

impl rodio::Source for Pcm {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        RATE
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        kill(&self.child);
    }
}

const FPS: f64 = 30.0;

pub struct Player {
    pub path: PathBuf,
    pub meta: Option<Meta>,
    meta_rx: Option<Receiver<Option<Meta>>>,
    /// Tamanho do quadro decodificado (menor que a fonte, para ficar leve).
    fw: u32,
    fh: u32,
    max: (u32, u32),
    tex: Option<TextureHandle>,
    pub pos: f64,
    session: Option<Session>,
    /// Parar de tocar aqui (fim do trecho escolhido).
    pub stop_at: Option<f64>,
    seek_rx: Option<Receiver<Option<Frame>>>,
    seek_want: Option<f64>,
    failed: bool,
    audio_on: bool,
    volume: f32,
}

impl Player {
    pub fn new(path: impl Into<PathBuf>, at: f64, max: (u32, u32)) -> Self {
        let path = path.into();
        let (tx, rx) = mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(probe_sync(&p));
        });
        Self {
            path,
            meta: None,
            meta_rx: Some(rx),
            fw: 0,
            fh: 0,
            max,
            tex: None,
            pos: at.max(0.0),
            session: None,
            stop_at: None,
            seek_rx: None,
            seek_want: Some(at.max(0.0)),
            failed: false,
            audio_on: false,
            volume: 1.0,
        }
    }

    /// Toca o som junto com o vídeo.
    pub fn with_audio(mut self) -> Self {
        self.audio_on = true;
        self
    }

    /// Volume de 0 a 1; vale na hora, mesmo tocando.
    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        if let Some(a) = self.session.as_ref().and_then(|s| s.audio.as_ref()) {
            a.sink.set_volume(self.volume);
        }
    }

    pub fn duration(&self) -> f64 {
        self.meta.map(|m| m.duration).unwrap_or(0.0)
    }

    pub fn aspect(&self) -> f32 {
        match self.meta {
            Some(m) if m.width > 0 && m.height > 0 => m.width as f32 / m.height as f32,
            _ => 16.0 / 9.0,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.session.is_some()
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn texture(&self) -> Option<&TextureHandle> {
        self.tex.as_ref()
    }

    pub fn seek(&mut self, t: f64) {
        let d = self.duration();
        let t = if d > 0.0 { t.clamp(0.0, (d - 0.05).max(0.0)) } else { t.max(0.0) };
        self.session = None;
        self.pos = t;
        self.seek_want = Some(t);
    }

    pub fn play(&mut self) {
        let Some(_m) = self.meta else { return };
        let end = self.stop_at.unwrap_or(self.duration());
        if self.pos >= end - 0.05 {
            self.pos = 0.0;
        }
        self.session = self.spawn_stream(self.pos, end);
    }

    pub fn pause(&mut self) {
        self.session = None;
    }

    #[allow(dead_code)]
    pub fn toggle(&mut self) {
        if self.is_playing() { self.pause() } else { self.play() }
    }

    /// Avança o estado. Chamar uma vez por quadro antes de desenhar.
    pub fn update(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.meta_rx {
            match rx.try_recv() {
                Ok(m) => {
                    self.meta_rx = None;
                    match m {
                        Some(m) if m.width > 0 => {
                            self.meta = Some(m);
                            let s = (self.max.0 as f64 / m.width as f64).min(self.max.1 as f64 / m.height as f64).min(1.0);
                            self.fw = ((m.width as f64 * s) as u32).max(2) & !1;
                            self.fh = ((m.height as f64 * s) as u32).max(2) & !1;
                        }
                        _ => self.failed = true,
                    }
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(30)),
                Err(TryRecvError::Disconnected) => {
                    self.meta_rx = None;
                    self.failed = true;
                }
            }
            if self.meta.is_none() {
                return;
            }
        }

        // Busca com o player parado: um quadro por vez, o pedido mais novo vence.
        if let Some(rx) = &self.seek_rx {
            match rx.try_recv() {
                Ok(f) => {
                    self.seek_rx = None;
                    if let Some(f) = f {
                        self.show(ctx, f.rgba);
                    }
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(16)),
                Err(TryRecvError::Disconnected) => self.seek_rx = None,
            }
        }
        if self.seek_rx.is_none() && self.session.is_none() {
            if let Some(t) = self.seek_want.take() {
                self.seek_rx = Some(self.spawn_single(t));
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }

        let mut latest: Option<Vec<u8>> = None;
        let mut finished = false;
        if let Some(s) = &mut self.session {
            let end = self.stop_at.unwrap_or(f64::MAX);
            loop {
                if s.pending.is_none() {
                    match s.rx.try_recv() {
                        Ok(f) => {
                            if s.started.is_none() {
                                s.started = Some(Instant::now());
                            }
                            s.pending = Some(f);
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            finished = true;
                            break;
                        }
                    }
                }
                let target = s.from + s.started.map(|i| i.elapsed().as_secs_f64()).unwrap_or(0.0);
                match &s.pending {
                    Some(f) if f.t <= target => {
                        let f = s.pending.take().unwrap();
                        self.pos = f.t;
                        latest = Some(f.rgba);
                    }
                    _ => break,
                }
            }
            if self.pos >= end - 0.5 / FPS {
                finished = true;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
        }
        if let Some(rgba) = latest {
            self.show(ctx, rgba);
        }
        if finished {
            self.session = None;
        }
    }

    fn show(&mut self, ctx: &egui::Context, rgba: Vec<u8>) {
        if rgba.len() != (self.fw * self.fh * 4) as usize {
            return;
        }
        let img = ColorImage::from_rgba_unmultiplied([self.fw as usize, self.fh as usize], &rgba);
        match &mut self.tex {
            Some(t) => t.set(img, TextureOptions::LINEAR),
            None => self.tex = Some(ctx.load_texture(format!("player:{}", self.path.display()), img, TextureOptions::LINEAR)),
        }
    }

    fn base_cmd(&self, at: f64) -> Option<Command> {
        let mut c = ffmpeg()?;
        c.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        if at > 0.0 {
            c.args(["-ss", &format!("{at:.3}")]);
        }
        c.arg("-i").arg(&self.path);
        Some(c)
    }

    fn spawn_single(&self, at: f64) -> Receiver<Option<Frame>> {
        let (tx, rx) = mpsc::channel();
        let cmd = self.base_cmd(at);
        let (fw, fh) = (self.fw, self.fh);
        std::thread::spawn(move || {
            let frame = (|| {
                let mut c = cmd?;
                let out = c
                    .args(["-frames:v", "1", "-an", "-vf", &format!("scale={fw}:{fh}:flags=bilinear"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    .output()
                    .ok()?;
                (out.stdout.len() == (fw * fh * 4) as usize).then(|| Frame { t: at, rgba: out.stdout })
            })();
            let _ = tx.send(frame);
        });
        rx
    }

    fn spawn_stream(&self, from: f64, end: f64) -> Option<Session> {
        let mut c = self.base_cmd(from)?;
        let len = (end - from).max(0.1);
        let (fw, fh) = (self.fw, self.fh);
        let mut child = c
            .args(["-t", &format!("{len:.3}"), "-an", "-vf", &format!("fps={FPS},scale={fw}:{fh}:flags=bilinear"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let child = Arc::new(Mutex::new(Some(child)));
        let (tx, rx) = mpsc::sync_channel::<Frame>(6);
        std::thread::spawn(move || {
            let size = (fw * fh * 4) as usize;
            let mut n = 0u64;
            loop {
                let mut buf = vec![0u8; size];
                if stdout.read_exact(&mut buf).is_err() {
                    break;
                }
                if tx.send(Frame { t: from + n as f64 / FPS, rgba: buf }).is_err() {
                    break;
                }
                n += 1;
            }
        });
        let audio = if self.audio_on { self.spawn_audio(from, len) } else { None };
        Some(Session { rx, child, from, started: None, pending: None, audio })
    }

    fn spawn_audio(&self, from: f64, len: f64) -> Option<Audio> {
        let (out, handle) = rodio::OutputStream::try_default().ok()?;
        let sink = rodio::Sink::try_new(&handle).ok()?;
        let mut c = self.base_cmd(from)?;
        let mut child = c
            .args(["-t", &format!("{len:.3}"), "-vn", "-ac", "2", "-ar", &RATE.to_string(), "-f", "f32le", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let child = Arc::new(Mutex::new(Some(child)));
        let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(16);
        std::thread::spawn(move || {
            // 4096 quadros estéreo por bloco; só blocos inteiros.
            let mut raw = vec![0u8; 4096 * 2 * 4];
            loop {
                let mut got = 0;
                while got < raw.len() {
                    match stdout.read(&mut raw[got..]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got += n,
                    }
                }
                let whole = got / 8 * 8;
                if whole > 0 {
                    let b: Vec<f32> = raw[..whole].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                    if tx.send(b).is_err() {
                        break;
                    }
                }
                if got < raw.len() {
                    break;
                }
            }
        });
        sink.set_volume(self.volume);
        sink.append(Pcm { rx, buf: Vec::new(), i: 0 });
        Some(Audio { _out: out, sink, child })
    }
}

// ------------------------------------------------------------- forma de onda

/// Envelope do áudio: `bins` picos de 0 a 1 (normalizados pelo maior).
/// Bloqueante; chamar fora da UI. `None` se o arquivo não tem áudio.
pub fn peaks_sync(path: &Path, bins: usize) -> Option<Vec<f32>> {
    let out = ffmpeg()?
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "2000", "-f", "s16le", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let samples: Vec<i16> = out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    if samples.is_empty() || bins == 0 {
        return None;
    }
    let per = (samples.len() as f64 / bins as f64).max(1.0);
    let mut v: Vec<f32> = (0..bins)
        .map(|i| {
            let a = (i as f64 * per) as usize;
            let b = (((i + 1) as f64 * per) as usize).min(samples.len()).max(a + 1);
            samples[a.min(samples.len() - 1)..b].iter().map(|s| (*s as f32).abs()).fold(0.0, f32::max)
        })
        .collect();
    let max = v.iter().cloned().fold(0.0, f32::max).max(1.0);
    v.iter_mut().for_each(|x| *x /= max);
    Some(v)
}

/// Picos em segundo plano: a UI lê o resultado quando chegar.
pub fn peaks(path: &Path, bins: usize) -> Receiver<Option<Vec<f32>>> {
    let (tx, rx) = mpsc::channel();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(peaks_sync(&p, bins));
    });
    rx
}
