// Formatação comum. Timecode é a assinatura visual do FLAXY: toda duração e
// posição aparece como HH:MM:SS:FF.

const FPS = 30;

export function timecode(seconds: number, frames = true) {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const p = (n: number) => String(n).padStart(2, "0");
  const base = `${p(h)}:${p(m)}:${p(s)}`;
  return frames ? `${base}:${p(Math.floor((seconds - total) * FPS))}` : base;
}

export function bytes(n: number) {
  if (!n) return "0 MB";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${n.toLocaleString("pt-BR", { maximumFractionDigits: i >= 3 ? 2 : 0 })} ${units[i]}`;
}

export function dateTime(iso: string) {
  const d = new Date(iso);
  return d.toLocaleString("pt-BR", { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" });
}

export function ratioLabel(w: number, h: number) {
  const g = (a: number, b: number): number => (b ? g(b, a % b) : a);
  const d = g(w, h);
  return `${w / d}:${h / d}`;
}

export const STATUS_LABEL: Record<string, string> = {
  queued: "Na fila",
  processing: "Renderizando",
  done: "Pronto",
  error: "Falhou",
  canceled: "Cancelado",
};
