//! Motor de composição: monta e executa o comando FFmpeg de um vídeo.
//!
//! Camada 0: fundo do template (vídeo ou imagem), escalado para a saída.
//! Camada 1: vídeo bruto, recortado, cortado no tempo e encaixado na área.
//! Tudo num único filtergraph e numa única passada de encode.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use crate::media::{self, Encoder};
use crate::models::{ItemOverrides, Template};
use crate::{Error, Result};

/// Parâmetros já resolvidos (template + ajustes do item) para UM vídeo.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub output_w: i64,
    pub output_h: i64,
    pub overlay_x: i64,
    pub overlay_y: i64,
    pub overlay_w: i64,
    pub overlay_h: i64,
    pub cover: bool,
    pub crop_x: f64,
    pub crop_y: f64,
    pub crop_w: f64,
    pub crop_h: f64,
    pub trim_start: f64,
    pub trim_end: Option<f64>,
}

impl Settings {
    pub fn resolve(tpl: &Template, o: &ItemOverrides) -> Self {
        let fit = o.fit_mode.as_deref().unwrap_or(&tpl.fit_mode);
        let mut s = Self {
            output_w: tpl.output_w,
            output_h: tpl.output_h,
            overlay_x: o.overlay_x.unwrap_or(tpl.overlay_x),
            overlay_y: o.overlay_y.unwrap_or(tpl.overlay_y),
            overlay_w: o.overlay_w.unwrap_or(tpl.overlay_w),
            overlay_h: o.overlay_h.unwrap_or(tpl.overlay_h),
            cover: fit != "contain",
            crop_x: o.crop_x,
            crop_y: o.crop_y,
            crop_w: o.crop_w,
            crop_h: o.crop_h,
            trim_start: o.trim_start.unwrap_or(0.0),
            trim_end: o.trim_end,
        };
        s.clamp();
        s
    }

    /// Corrige valores impossíveis em vez de deixar o FFmpeg falhar com um
    /// erro de filtergraph ilegível.
    fn clamp(&mut self) {
        self.overlay_w = self.overlay_w.clamp(2, self.output_w) & !1;
        self.overlay_h = self.overlay_h.clamp(2, self.output_h) & !1;
        self.overlay_x = self.overlay_x.clamp(0, self.output_w - self.overlay_w);
        self.overlay_y = self.overlay_y.clamp(0, self.output_h - self.overlay_h);

        self.crop_x = self.crop_x.clamp(0.0, 0.99);
        self.crop_y = self.crop_y.clamp(0.0, 0.99);
        self.crop_w = self.crop_w.clamp(0.01, 1.0 - self.crop_x);
        self.crop_h = self.crop_h.clamp(0.01, 1.0 - self.crop_y);

        self.trim_start = self.trim_start.max(0.0);
        if let Some(end) = self.trim_end {
            if end <= self.trim_start + 0.05 {
                self.trim_end = None;
            }
        }
    }

    fn crops_source(&self) -> bool {
        (self.crop_x, self.crop_y, self.crop_w, self.crop_h) != (0.0, 0.0, 1.0, 1.0)
    }

    /// Duração do trecho da fonte que entra na saída, se conhecida.
    pub fn clip_duration(&self, source_duration: f64) -> f64 {
        let end = self.trim_end.unwrap_or(source_duration).min(source_duration.max(0.0));
        (end - self.trim_start).max(0.0)
    }
}

/// Filtergraph de vídeo. `end_with_raw` faz o overlay terminar com o vídeo bruto.
pub fn video_filter(s: &Settings, end_with_raw: bool) -> String {
    let (ow, oh) = (s.output_w, s.output_h);
    let (ax, ay, aw, ah) = (s.overlay_x, s.overlay_y, s.overlay_w, s.overlay_h);

    // Recorte com expressões relativas (iw/ih): vale para qualquer resolução.
    let crop = if s.crops_source() {
        format!(
            "crop=iw*{:.6}:ih*{:.6}:iw*{:.6}:ih*{:.6},",
            s.crop_w, s.crop_h, s.crop_x, s.crop_y
        )
    } else {
        String::new()
    };

    let fit = if s.cover {
        format!("scale={aw}:{ah}:force_original_aspect_ratio=increase,crop={aw}:{ah}")
    } else {
        format!("scale={aw}:{ah}:force_original_aspect_ratio=decrease,pad={aw}:{ah}:(ow-iw)/2:(oh-ih)/2:black")
    };

    let shortest = if end_with_raw { ":shortest=1" } else { "" };
    format!(
        "[0:v]scale={ow}:{oh},setsar=1[bg];\
         [1:v]{crop}{fit},setsar=1[raw];\
         [bg][raw]overlay={ax}:{ay}{shortest},format=yuv420p[out]"
    )
}

pub struct ComposeJob<'a> {
    pub template: &'a Template,
    pub input: &'a Path,
    pub output: &'a Path,
    pub log: Option<PathBuf>,
    pub overrides: &'a ItemOverrides,
    /// Arquivo .ass a queimar. O FFmpeg roda na pasta dele (nome simples no filtro).
    pub subtitles: Option<PathBuf>,
}

/// Duração da saída: a do fundo na regra "template", senão a do trecho do bruto.
pub fn output_duration(tpl: &Template, s: &Settings, raw: &media::Probe, bg: &media::Probe) -> f64 {
    let rule_template =
        tpl.duration_rule == "template" && !media::is_image(Path::new(&tpl.file_path));
    if rule_template && bg.duration > 0.0 {
        bg.duration
    } else {
        s.clip_duration(raw.duration)
    }
}

/// Monta os argumentos do FFmpeg. Separado da execução para ser testável.
pub fn build_args(
    job: &ComposeJob<'_>,
    s: &Settings,
    enc: &Encoder,
    raw: &media::Probe,
    bg: &media::Probe,
) -> Vec<String> {
    let tpl = job.template;
    let bg_path = Path::new(&tpl.file_path);
    let bg_is_image = media::is_image(bg_path);

    // Fundo em imagem não tem duração própria: segue sempre o vídeo bruto.
    let rule = if bg_is_image { "raw" } else { tpl.duration_rule.as_str() };
    let end_with_raw = rule != "template";
    // Fundo infinito (imagem ou loop): o overlay precisa terminar com o bruto.
    // Fundo em vídeo no modo "raw" congela o último quadro (eof_action=repeat),
    // como no backend antigo.
    let bg_infinite = bg_is_image || rule == "loop_template";

    let mut a: Vec<String> = vec!["-hide_banner".into(), "-y".into(), "-nostats".into()];
    a.extend(["-progress".into(), "pipe:1".into()]);

    if bg_is_image {
        a.extend(["-loop".into(), "1".into()]);
    } else if rule == "loop_template" {
        a.extend(["-stream_loop".into(), "-1".into()]);
    }
    a.extend(["-i".into(), tpl.file_path.clone()]);

    if s.trim_start > 0.0 {
        a.extend(["-ss".into(), format!("{:.3}", s.trim_start)]);
    }
    if let Some(end) = s.trim_end {
        a.extend(["-t".into(), format!("{:.3}", end - s.trim_start)]);
    }
    a.extend(["-i".into(), job.input.to_string_lossy().into_owned()]);

    // Faixa extra (input 2). Em loop: uma música curta cobre o vídeo inteiro;
    // o fim vem de -shortest / -t, como o resto da saída.
    let o = job.overrides;
    let extra = o.audio_path.as_deref().filter(|p| Path::new(p).is_file());
    if let Some(p) = extra {
        a.extend(["-stream_loop".into(), "-1".into()]);
        if let Some(off) = o.audio_offset.filter(|v| *v > 0.0) {
            a.extend(["-ss".into(), format!("{off:.3}")]);
        }
        a.extend(["-i".into(), p.to_string()]);
    }

    let mut graph = video_filter(s, bg_infinite);
    if let Some(name) = job.subtitles.as_ref().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        graph = graph.replacen(",format=yuv420p[out]", &format!(",ass={name},format=yuv420p[out]"), 1);
    }

    // Áudio base. O fundo em loop tem áudio infinito: a mixagem termina com o bruto.
    let use_tpl_audio = bg.has_audio && !bg_is_image;
    let base: Option<&str> = match tpl.audio_source.as_str() {
        "template" if use_tpl_audio => Some("[0:a]"),
        "both" if use_tpl_audio && raw.has_audio => {
            graph.push_str(&format!(
                ";[1:a]volume={:.3}[a1];[0:a]volume={:.3}[a0];\
                 [a1][a0]amix=inputs=2:duration=first:normalize=0[aout]",
                tpl.audio_mix_raw, tpl.audio_mix_template
            ));
            Some("[aout]")
        }
        _ if raw.has_audio => Some("[1:a]"),
        _ => None,
    };

    let out_dur = output_duration(tpl, s, raw, bg);
    let fade = if o.audio_fade.unwrap_or(false) && out_dur > 4.0 {
        format!(",afade=t=out:st={:.3}:d=2", out_dur - 2.0)
    } else {
        String::new()
    };
    let orig_vol = o.original_volume.unwrap_or(1.0).clamp(0.0, 4.0);
    let extra_vol = o.audio_volume.unwrap_or(1.0).clamp(0.0, 4.0);

    let audio_map: Option<String> = match (base, extra.is_some()) {
        (Some(b), true) => {
            graph.push_str(&format!(
                ";{b}volume={orig_vol:.3}[ab];[2:a]volume={extra_vol:.3}[ae];\
                 [ab][ae]amix=inputs=2:duration=first:normalize=0{fade}[afinal]"
            ));
            Some("[afinal]".into())
        }
        (None, true) => {
            graph.push_str(&format!(";[2:a]volume={extra_vol:.3}{fade}[afinal]"));
            Some("[afinal]".into())
        }
        (Some(b), false) if orig_vol != 1.0 || !fade.is_empty() => {
            graph.push_str(&format!(";{b}volume={orig_vol:.3}{fade}[afinal]"));
            Some("[afinal]".into())
        }
        (Some(b), false) => Some(b.trim_matches(['[', ']']).to_string()),
        (None, false) => None,
    };

    a.extend(["-filter_complex".into(), graph, "-map".into(), "[out]".into()]);
    if let Some(m) = audio_map {
        a.extend(["-map".into(), m, "-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
    }

    if end_with_raw {
        a.push("-shortest".into());
    } else if bg.duration > 0.0 {
        a.extend(["-t".into(), format!("{:.3}", bg.duration)]);
    }

    a.extend(["-c:v".into(), enc.name.into()]);
    if let Some(p) = enc.preset {
        a.extend(["-preset".into(), p.into()]);
    }
    a.extend(["-b:v".into(), tpl.video_bitrate.clone()]);
    if tpl.output_format == "mp4" || tpl.output_format == "mov" {
        a.extend(["-movflags".into(), "+faststart".into()]);
    }
    a.push(job.output.to_string_lossy().into_owned());
    a
}

/// Executa a composição. `on_progress` recebe 0..=99 durante o encode.
pub async fn compose<F>(job: ComposeJob<'_>, on_progress: F) -> Result<()>
where
    F: FnMut(i64) + Send,
{
    let ff = media::ffmpeg().ok_or_else(|| Error::Ffmpeg("ffmpeg não encontrado no PATH".into()))?;
    let s = Settings::resolve(job.template, job.overrides);
    let enc = media::encoder().await;
    let (raw, bg) = tokio::join!(
        media::probe(job.input),
        media::probe(Path::new(&job.template.file_path))
    );

    let total = output_duration(job.template, &s, &raw, &bg).max(0.1);

    let args = build_args(&job, &s, enc, &raw, &bg);
    let cwd = job.subtitles.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf);
    run_ffmpeg(ff, &args, cwd.as_deref(), total, job.log.as_deref(), on_progress).await
}

/// Roda o FFmpeg com `-progress pipe:1` nos argumentos e reporta 0..=99.
/// Grava comando + stderr em `log`; em falha devolve as últimas linhas do stderr.
pub async fn run_ffmpeg<F>(ff: &Path, args: &[String], cwd: Option<&Path>, total: f64, log: Option<&Path>, mut on_progress: F) -> Result<()>
where
    F: FnMut(i64) + Send,
{
    let total = total.max(0.1);
    let mut cmd = Command::new(ff);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let mut stderr = child.stderr.take().expect("stderr");
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
        buf
    });

    let stdout = child.stdout.take().expect("stdout");
    let mut lines = BufReader::new(stdout).lines();
    let mut last = -1;
    while let Some(line) = lines.next_line().await? {
        if let Some(us) = line.strip_prefix("out_time_us=") {
            if let Ok(us) = us.trim().parse::<f64>() {
                let pct = ((us / 1_000_000.0 / total) * 100.0).clamp(0.0, 99.0) as i64;
                if pct != last {
                    last = pct;
                    on_progress(pct);
                }
            }
        }
    }

    let status = child.wait().await?;
    let stderr = stderr_task.await.unwrap_or_default();
    if let Some(log) = log {
        let mut content = format!("ffmpeg {}\n\n", args.join(" ")).into_bytes();
        content.extend_from_slice(&stderr);
        let _ = tokio::fs::write(log, content).await;
    }
    if !status.success() {
        let text = String::from_utf8_lossy(&stderr);
        let tail: String = text
            .lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(Error::Ffmpeg(format!("código {:?}: {tail}", status.code())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tpl() -> Template {
        Template {
            id: 1,
            name: "t".into(),
            file_path: "bg.png".into(),
            overlay_x: 0,
            overlay_y: 0,
            overlay_w: 1080,
            overlay_h: 1920,
            fit_mode: "cover".into(),
            output_w: 1080,
            output_h: 1920,
            output_format: "mp4".into(),
            video_bitrate: "8M".into(),
            audio_source: "raw".into(),
            audio_mix_raw: 1.0,
            audio_mix_template: 0.5,
            duration_rule: "raw".into(),
            created_at: String::new(),
        }
    }

    #[test]
    fn clamp_fixes_impossible_values() {
        let o = ItemOverrides {
            crop_x: 0.9,
            crop_w: 0.5,
            overlay_w: Some(5000),
            overlay_x: Some(300),
            trim_start: Some(5.0),
            trim_end: Some(4.0),
            ..Default::default()
        };
        let s = Settings::resolve(&tpl(), &o);
        assert_eq!(s.overlay_w, 1080);
        assert_eq!(s.overlay_x, 0);
        assert!((s.crop_x + s.crop_w - 1.0).abs() < 1e-9);
        assert_eq!(s.trim_end, None);
    }

    #[test]
    fn filter_has_relative_crop() {
        let o = ItemOverrides { crop_x: 0.25, crop_w: 0.5, ..Default::default() };
        let s = Settings::resolve(&tpl(), &o);
        let f = video_filter(&s, true);
        assert!(f.contains("crop=iw*0.500000:ih*1.000000:iw*0.250000:ih*0.000000"));
        assert!(f.contains("shortest=1"));
    }

    #[test]
    fn extra_audio_mixes_with_original() {
        let t = tpl();
        // Qualquer arquivo existente serve: o teste só monta os argumentos.
        let fake = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml").to_string();
        let o = ItemOverrides {
            audio_path: Some(fake.clone()),
            audio_volume: Some(0.4),
            original_volume: Some(0.0),
            audio_fade: Some(true),
            ..Default::default()
        };
        let s = Settings::resolve(&t, &o);
        let job = ComposeJob {
            template: &t,
            input: Path::new("in.mp4"),
            output: Path::new("out.mp4"),
            log: None,
            overrides: &o,
            subtitles: None,
        };
        let enc = Encoder { name: "libx264", preset: None };
        let raw = media::Probe { duration: 10.0, has_audio: true };
        let joined = build_args(&job, &s, &enc, &raw, &media::Probe::default()).join(" ");
        assert!(joined.contains(&format!("-stream_loop -1 -i {fake}")));
        assert!(joined.contains("[1:a]volume=0.000[ab];[2:a]volume=0.400[ae]"));
        assert!(joined.contains("afade=t=out:st=8.000:d=2[afinal]"));
        assert!(joined.contains("-map [afinal]"));
        assert!(joined.contains("-shortest"));
    }

    #[test]
    fn trim_args_before_raw_input() {
        let t = tpl();
        let o = ItemOverrides { trim_start: Some(2.0), trim_end: Some(7.5), ..Default::default() };
        let s = Settings::resolve(&t, &o);
        let job = ComposeJob {
            template: &t,
            input: Path::new("in.mp4"),
            output: Path::new("out.mp4"),
            log: None,
            overrides: &o,
            subtitles: None,
        };
        let enc = Encoder { name: "libx264", preset: Some("veryfast") };
        let raw = media::Probe { duration: 10.0, has_audio: true };
        let args = build_args(&job, &s, &enc, &raw, &media::Probe::default());
        let joined = args.join(" ");
        assert!(joined.contains("-ss 2.000 -t 5.500 -i in.mp4"));
        assert!(joined.contains("-map 1:a"));
        assert!((s.clip_duration(10.0) - 5.5).abs() < 1e-9);
    }
}
