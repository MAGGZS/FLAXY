// Cliente da API Rust. Mesma origem em produção; proxy do Vite em dev.

const BASE = ((import.meta.env.VITE_API_URL as string | undefined) ?? "").replace(/\/$/, "");
const KEY = import.meta.env.VITE_API_KEY as string | undefined;

export const apiUrl = (path: string) => {
  const url = `${BASE}/api${path}`;
  if (!KEY) return url;
  return url + (url.includes("?") ? "&" : "?") + "key=" + encodeURIComponent(KEY);
};

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const init: RequestInit = { method, headers: {} };
  if (KEY) (init.headers as Record<string, string>)["x-flaxy-key"] = KEY;
  if (body instanceof FormData) init.body = body;
  else if (body !== undefined) {
    init.body = JSON.stringify(body);
    (init.headers as Record<string, string>)["content-type"] = "application/json";
  }
  let res: Response;
  try {
    res = await fetch(`${BASE}/api${path}`, init);
  } catch {
    throw new ApiError(0, "Servidor fora do ar");
  }
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new ApiError(res.status, data?.detail ?? `Erro ${res.status}`);
  return data as T;
}

export const api = {
  get: <T>(p: string) => request<T>("GET", p),
  post: <T>(p: string, b?: unknown) => request<T>("POST", p, b),
  put: <T>(p: string, b?: unknown) => request<T>("PUT", p, b),
  del: <T>(p: string) => request<T>("DELETE", p),
};

export function wsUrl() {
  const base = BASE || window.location.origin;
  const u = new URL(apiUrl("/ws"), base);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  return u.toString();
}

/** Upload com progresso real (fetch não expõe progresso de envio). */
export function uploadFiles(path: string, files: File[], field: string, onProgress: (pct: number) => void, extra?: Record<string, string>) {
  return new Promise<any>((resolve, reject) => {
    const fd = new FormData();
    if (extra) for (const [k, v] of Object.entries(extra)) fd.append(k, v);
    for (const f of files) fd.append(field, f, f.name);
    const xhr = new XMLHttpRequest();
    xhr.open("POST", `${BASE}/api${path}`);
    if (KEY) xhr.setRequestHeader("x-flaxy-key", KEY);
    xhr.upload.onprogress = (e) => e.lengthComputable && onProgress(Math.round((e.loaded / e.total) * 100));
    xhr.onload = () => {
      let data: any = {};
      try { data = JSON.parse(xhr.responseText); } catch { /* corpo vazio */ }
      if (xhr.status >= 200 && xhr.status < 300) resolve(data);
      else reject(new ApiError(xhr.status, data?.detail ?? `Erro ${xhr.status}`));
    };
    xhr.onerror = () => reject(new ApiError(0, "Servidor fora do ar"));
    xhr.send(fd);
  });
}

// ------------------------------------------------------------------ tipos

export type Template = {
  id: number;
  name: string;
  file_path: string;
  overlay_x: number;
  overlay_y: number;
  overlay_w: number;
  overlay_h: number;
  fit_mode: "cover" | "contain";
  output_w: number;
  output_h: number;
  output_format: string;
  video_bitrate: string;
  audio_source: "raw" | "template" | "both";
  audio_mix_raw: number;
  audio_mix_template: number;
  duration_rule: "raw" | "template" | "loop_template";
  created_at: string;
};

export type TemplateInput = Omit<Template, "id" | "file_path" | "created_at">;

export type Overrides = {
  crop_x: number;
  crop_y: number;
  crop_w: number;
  crop_h: number;
  fit_mode?: "cover" | "contain" | null;
  trim_start?: number | null;
  trim_end?: number | null;
  caption_style_id?: number | null;
  transcript_id?: number | null;
};

export type JobItem = {
  id: number;
  job_id: number;
  input_path: string;
  output_path: string | null;
  status: Status;
  progress: number;
  error_msg: string | null;
};

export type Status = "queued" | "processing" | "done" | "error" | "canceled";

export type Job = {
  id: number;
  template_id: number;
  status: Status;
  canceled: boolean;
  created_at: string;
  items: JobItem[];
};

export type Uploaded = {
  path: string;
  file: string;
  name: string;
  duration: number;
  has_audio: boolean;
};

export type OutputFile = { name: string; size_bytes: number; modified_at: string };

export type Health = {
  status: string;
  version: string;
  ffmpeg: boolean;
  encoder: string;
  max_videos_per_job: number;
  max_upload_mb: number;
};

export type Stats = {
  items_by_status: Partial<Record<Status, number>>;
  done_last_7_days: { date: string; count: number }[];
  templates: number;
  disk_bytes: { uploads: number; output: number; templates: number };
};

export const basename = (p: string) => p.split(/[\\/]/).pop() ?? p;

// ------------------------------------------------------------ IA / legendas

export type Provider = "local" | "groq" | "openai" | "deepgram" | "assemblyai";

export type AiSettings = {
  transcriber: Provider;
  language: string;
  local_model: string;
  llm_provider: "" | "anthropic" | "openai" | "groq" | "ollama";
  llm_model: string;
  ollama_url: string;
  keys: { groq: string; openai: string; deepgram: string; assemblyai: string; anthropic: string };
};

export type WhisperModel = { name: string; size_mb: number; installed: boolean; progress: number | null; error: string | null };

export type Word = { text: string; start: number; end: number };

export type Transcript = {
  id: number;
  source_path: string;
  provider: string;
  status: "queued" | "processing" | "done" | "error";
  error_msg: string | null;
  words: Word[];
};

export type CaptionKind = "highlight" | "word" | "classic" | "box";

export type CaptionStyle = {
  kind: CaptionKind;
  font: string;
  size: number;
  bold: boolean;
  uppercase: boolean;
  color: string;
  highlight: string;
  outline_color: string;
  outline: number;
  shadow: number;
  back_color: string;
  back_opacity: number;
  position: "bottom" | "center" | "top";
  margin: number;
  max_words: number;
  max_chars: number;
};

export type StyleRow = { id: number; name: string; builtin: boolean; style: CaptionStyle };

// ------------------------------------------------------------------ clipes

export type Rect = { x: number; y: number; w: number; h: number };
export type ClipLayout = "vertical_camera" | "vertical_split" | "horizontal";

export type ClipProject = {
  id: number;
  url: string;
  title: string;
  source_path: string | null;
  duration: number;
  width: number;
  height: number;
  status: "downloading" | "transcribing" | "analyzing" | "ready" | "error";
  progress: number;
  error_msg: string | null;
  transcript_id: number | null;
  min_len: number;
  max_len: number;
  layout: ClipLayout;
  created_at: string;
};

export type Clip = {
  id: number;
  project_id: number;
  start: number;
  end: number;
  score: number;
  title: string;
  reason: string;
  approved: boolean;
  layout: ClipLayout;
  caption_style_id: number | null;
  status: "none" | "queued" | "processing" | "done" | "error";
  progress: number;
  output_path: string | null;
  error_msg: string | null;
  camera: Rect;
  screen: Rect;
};

export type ClipProjectView = ClipProject & { clips: Clip[] };
