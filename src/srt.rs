//! Файлы субтитров SubRip (.srt): чтение и запись.

/// Одна реплика: когда она на экране и что в ней написано.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Текст файла субтитров. Старые файлы с русским текстом часто записаны не в UTF-8,
/// а в кодировке Windows-1251 или в UTF-16.
pub fn decode(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = body.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    bytes
        .iter()
        .map(|&b| match b {
            0..=0x7F => b as char,
            0xA8 => 'Ё',
            0xB8 => 'ё',
            0xAB => '«',
            0xBB => '»',
            0x85 => '…',
            0x96 | 0x97 => '—',
            0xC0..=0xFF => char::from_u32(0x410 + (b - 0xC0) as u32).unwrap_or('?'),
            _ => ' ',
        })
        .collect()
}

/// Время в секундах из записи `ЧЧ:ММ:СС,МММ`; дробную часть отделяют и точкой.
fn parse_time(s: &str) -> Option<f64> {
    let (clock, millis) = s.trim().split_once([',', '.']).unwrap_or((s.trim(), "0"));
    let mut seconds = 0.0;
    for part in clock.split(':') {
        seconds = seconds * 60.0 + part.trim().parse::<u32>().ok()? as f64;
    }
    let digits = millis.trim();
    let fraction = digits.parse::<u32>().ok()? as f64 / 10f64.powi(digits.len() as i32);
    Some(seconds + fraction)
}

fn format_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Разбирает файл субтитров. Реплики без времени или текста пропускаются, разметка в угловых
/// скобках (`<i>`, `<font …>`) убирается.
pub fn parse(source: &str) -> Vec<Cue> {
    let text = source.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();
    for block in text.split("\n\n") {
        let mut lines = block.lines().skip_while(|l| !l.contains("-->"));
        let Some((from, to)) = lines.next().and_then(|l| l.split_once("-->")) else { continue };
        // После времени конца могут идти координаты реплики: они не нужны.
        let to = to.split_whitespace().next().unwrap_or("");
        let (Some(start), Some(end)) = (parse_time(from), parse_time(to)) else { continue };
        let mut body = String::new();
        for line in lines {
            let mut inside = false;
            let plain: String = line
                .chars()
                .filter(|c| {
                    let tag = inside || *c == '<';
                    inside = (inside || *c == '<') && *c != '>';
                    !tag
                })
                .collect();
            if !plain.trim().is_empty() {
                if !body.is_empty() {
                    body.push('\n');
                }
                body += plain.trim();
            }
        }
        if !body.is_empty() && end > start {
            cues.push(Cue { start, end, text: body });
        }
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

/// Записывает реплики в формате SubRip.
pub fn write(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        out += &format!("{}\n{} --> {}\n{}\n\n", i + 1, format_time(cue.start), format_time(cue.end), cue.text.trim());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_is_parsed_with_tags_and_odd_line_endings() {
        let source = "\u{feff}1\r\n00:00:01,500 --> 00:00:03,000\r\nПривет, <i>мир</i>!\r\n\r\n\
                      2\n00:01:02.250 --> 00:01:04,000 X1:10 Y1:20\n<font color=\"#fff\">Вторая</font>\nстрока\n\n\
                      3\nне время\nмусор\n\n4\n00:00:09,000 --> 00:00:08,000\nконец раньше начала\n";
        assert_eq!(
            parse(source),
            vec![
                Cue { start: 1.5, end: 3.0, text: "Привет, мир!".into() },
                Cue { start: 62.25, end: 64.0, text: "Вторая\nстрока".into() },
            ]
        );
    }

    #[test]
    fn old_encodings_are_recognised() {
        assert_eq!(decode("Привет".as_bytes()), "Привет");
        // «Привет, ёж» в Windows-1251.
        assert_eq!(decode(&[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2, 0x2C, 0x20, 0xB8, 0xE6]), "Привет, ёж");
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Да".encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
        assert_eq!(decode(&utf16), "Да");
    }

    #[test]
    fn written_file_reads_back_the_same() {
        let cues = vec![
            Cue { start: 0.0, end: 2.04, text: "Раз".into() },
            Cue { start: 3661.007, end: 3662.5, text: "Два\nтри".into() },
        ];
        let text = write(&cues);
        assert!(text.starts_with("1\n00:00:00,000 --> 00:00:02,040\nРаз\n\n2\n01:01:01,007 --> 01:01:02,500\n"));
        let back = parse(&text);
        assert_eq!(back.len(), 2);
        for (a, b) in back.iter().zip(&cues) {
            assert!((a.start - b.start).abs() < 1e-9 && (a.end - b.end).abs() < 1e-9 && a.text == b.text, "{a:?}");
        }
    }
}
