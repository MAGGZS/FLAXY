use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Template: fundo fixo + área onde o vídeo bruto entra.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Template {
    pub id: i64,
    pub name: String,
    pub file_path: String,
    pub overlay_x: i64,
    pub overlay_y: i64,
    pub overlay_w: i64,
    pub overlay_h: i64,
    pub fit_mode: String,
    pub output_w: i64,
    pub output_h: i64,
    pub output_format: String,
    pub video_bitrate: String,
    pub audio_source: String,
    pub audio_mix_raw: f64,
    pub audio_mix_template: f64,
    pub duration_rule: String,
    pub created_at: String,
}

/// Campos editáveis de um template (tudo menos id, arquivo e data).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateInput {
    pub name: String,
    pub overlay_x: i64,
    pub overlay_y: i64,
    pub overlay_w: i64,
    pub overlay_h: i64,
    pub fit_mode: String,
    pub output_w: i64,
    pub output_h: i64,
    pub output_format: String,
    pub video_bitrate: String,
    pub audio_source: String,
    pub audio_mix_raw: f64,
    pub audio_mix_template: f64,
    pub duration_rule: String,
}

impl Default for TemplateInput {
    fn default() -> Self {
        Self {
            name: String::new(),
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
        }
    }
}

impl TemplateInput {
    /// Normaliza valores fora do domínio em vez de gravar lixo no banco.
    pub fn sanitize(mut self) -> Self {
        if !matches!(self.fit_mode.as_str(), "cover" | "contain") {
            self.fit_mode = "cover".into();
        }
        if !matches!(self.audio_source.as_str(), "raw" | "template" | "both") {
            self.audio_source = "raw".into();
        }
        if !matches!(self.duration_rule.as_str(), "raw" | "template" | "loop_template") {
            self.duration_rule = "raw".into();
        }
        if !matches!(self.output_format.as_str(), "mp4" | "mov" | "webm") {
            self.output_format = "mp4".into();
        }
        // O bitrate entra direto na linha de comando: só dígitos + sufixo k/M.
        let digits = self.video_bitrate.trim_end_matches(['k', 'K', 'M', 'm']);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            self.video_bitrate = "8M".into();
        }
        // Encoders H.264 exigem dimensões pares.
        self.output_w = self.output_w.clamp(16, 7680) & !1;
        self.output_h = self.output_h.clamp(16, 7680) & !1;
        self.audio_mix_raw = self.audio_mix_raw.clamp(0.0, 4.0);
        self.audio_mix_template = self.audio_mix_template.clamp(0.0, 4.0);
        self
    }
}

fn one() -> f64 {
    1.0
}

/// Ajustes de um único vídeo do lote. `None` = usa o template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemOverrides {
    /// Recorte da fonte em fração 0..1.
    #[serde(default)]
    pub crop_x: f64,
    #[serde(default)]
    pub crop_y: f64,
    #[serde(default = "one")]
    pub crop_w: f64,
    #[serde(default = "one")]
    pub crop_h: f64,
    #[serde(default)]
    pub overlay_x: Option<i64>,
    #[serde(default)]
    pub overlay_y: Option<i64>,
    #[serde(default)]
    pub overlay_w: Option<i64>,
    #[serde(default)]
    pub overlay_h: Option<i64>,
    #[serde(default)]
    pub fit_mode: Option<String>,
    /// Corte no tempo, em segundos da fonte.
    #[serde(default)]
    pub trim_start: Option<f64>,
    #[serde(default)]
    pub trim_end: Option<f64>,
    /// Faixa de áudio extra (música/narração), caminho dentro de uploads.
    #[serde(default)]
    pub audio_path: Option<String>,
    /// Volume da faixa extra (1.0 = original).
    #[serde(default)]
    pub audio_volume: Option<f64>,
    /// Volume do áudio que já vem do vídeo/fundo (0 = mudo).
    #[serde(default)]
    pub original_volume: Option<f64>,
    /// Início da faixa extra, em segundos da faixa.
    #[serde(default)]
    pub audio_offset: Option<f64>,
    /// Fade de saída no fim do vídeo.
    #[serde(default)]
    pub audio_fade: Option<bool>,
    /// Legenda: estilo + transcrição da fonte.
    #[serde(default)]
    pub caption_style_id: Option<i64>,
    #[serde(default)]
    pub transcript_id: Option<i64>,
}

impl Default for ItemOverrides {
    fn default() -> Self {
        Self {
            crop_x: 0.0,
            crop_y: 0.0,
            crop_w: 1.0,
            crop_h: 1.0,
            overlay_x: None,
            overlay_y: None,
            overlay_w: None,
            overlay_h: None,
            fit_mode: None,
            trim_start: None,
            trim_end: None,
            audio_path: None,
            audio_volume: None,
            original_volume: None,
            audio_offset: None,
            audio_fade: None,
            caption_style_id: None,
            transcript_id: None,
        }
    }
}

impl ItemOverrides {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Item de entrada em POST /jobs.
#[derive(Debug, Clone, Deserialize)]
pub struct JobItemInput {
    pub path: String,
    #[serde(flatten)]
    pub overrides: ItemOverrides,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobCreate {
    pub template_id: i64,
    pub items: Vec<JobItemInput>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct JobItem {
    pub id: i64,
    pub job_id: i64,
    pub input_path: String,
    #[serde(skip)]
    pub overrides: Option<String>,
    pub output_path: Option<String>,
    pub status: String,
    pub progress: i64,
    pub error_msg: Option<String>,
    pub log_path: Option<String>,
}

impl JobItem {
    pub fn overrides(&self) -> ItemOverrides {
        self.overrides
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct JobRow {
    pub id: i64,
    pub template_id: i64,
    pub status: String,
    pub canceled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    #[serde(flatten)]
    pub job: JobRow,
    pub items: Vec<JobItem>,
}

/// Evento de progresso enviado ao frontend pelo WebSocket.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub job_id: i64,
    pub item_id: i64,
    pub status: String,
    pub progress: i64,
}
