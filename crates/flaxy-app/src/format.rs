//! Formatação comum. Timecode é a assinatura visual do FLAXY: toda duração e
//! posição aparece como HH:MM:SS:FF.

const FPS: f64 = 30.0;

pub fn timecode(seconds: f64, frames: bool) -> String {
    let s = if seconds.is_finite() && seconds > 0.0 { seconds } else { 0.0 };
    let total = s.floor() as u64;
    let base = format!("{:02}:{:02}:{:02}", total / 3600, (total % 3600) / 60, total % 60);
    if frames {
        format!("{base}:{:02}", ((s - total as f64) * FPS).floor() as u64)
    } else {
        base
    }
}

pub fn bytes(n: u64) -> String {
    if n == 0 {
        return "0 MB".into();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i >= 3 {
        format!("{} {}", format!("{v:.2}").replace('.', ","), units[i])
    } else {
        format!("{v:.0} {}", units[i])
    }
}

const MONTHS: [&str; 12] = ["jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez"];
const MONTHS_LONG: [&str; 12] = [
    "Janeiro", "Fevereiro", "Março", "Abril", "Maio", "Junho", "Julho", "Agosto", "Setembro", "Outubro",
    "Novembro", "Dezembro",
];

/// "13 nov 14:05" no fuso local. Aceita o formato do SQLite e RFC 3339.
pub fn date_time(s: &str) -> String {
    use chrono::{Datelike, Local, NaiveDateTime, TimeZone, Timelike, Utc};
    let parsed = chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Local))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|n| Utc.from_utc_datetime(&n).with_timezone(&Local))
        });
    match parsed {
        Some(d) => format!("{:02} {} {:02}:{:02}", d.day(), MONTHS[d.month0() as usize], d.hour(), d.minute()),
        None => s.to_string(),
    }
}

pub fn month_title() -> String {
    use chrono::Datelike;
    let n = chrono::Local::now();
    format!("{} {}", MONTHS_LONG[n.month0() as usize], n.year())
}

pub fn ratio_label(w: i64, h: i64) -> String {
    fn g(a: i64, b: i64) -> i64 {
        if b == 0 { a } else { g(b, a % b) }
    }
    let d = g(w, h).max(1);
    format!("{}:{}", w / d, h / d)
}

pub fn status_label(s: &str) -> &'static str {
    match s {
        "queued" => "Na fila",
        "processing" => "Renderizando",
        "done" => "Pronto",
        "error" => "Falhou",
        "canceled" => "Cancelado",
        _ => "",
    }
}

pub fn thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

pub fn file_name(p: &str) -> String {
    std::path::Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tc() {
        assert_eq!(timecode(84.4, true), "00:01:24:12");
        assert_eq!(timecode(3600.0, false), "01:00:00");
        assert_eq!(thousands(1234567), "1.234.567");
        assert_eq!(ratio_label(1080, 1920), "9:16");
    }
}
