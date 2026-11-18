use std::{fmt, path::Path, str::FromStr};

use log::{error, warn};
use tokio::{
    fs::File,
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
};
use unicode_script::{Script, UnicodeScript};

const MAX_DUPLICATE_DURATION_MS: u64 = 50;
const MAX_LEN: usize = 200;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Subtitle {
    start: u64,
    end: u64,
    text: String,
}

impl Subtitle {
    fn new(s: &str, e: &str, t: &str) -> Self {
        Self {
            start: timestamp_to_millis(s),
            end: timestamp_to_millis(e),
            text: t.trim().to_string(),
        }
    }
}

impl fmt::Display for Subtitle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} --> {}\n{}",
            format_timestamp(self.start),
            format_timestamp(self.end),
            self.text
        )
    }
}

pub fn format_timestamp(millis: u64) -> String {
    let ms = millis % 1000;
    let total_s = millis / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;

    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms)
}

fn timestamp_to_millis(time: &str) -> u64 {
    let time = time.trim();

    let timestamp = if time.chars().filter(|&c| c == ':').count() == 1 {
        format!("00:{time}")
    } else {
        time.to_string()
    };

    let parts: Vec<&str> = timestamp.split([':', '.']).collect();

    if parts.len() != 4 {
        return 0;
    }

    let h = u64::from_str(parts[0]).unwrap_or(0);
    let m = u64::from_str(parts[1]).unwrap_or(0);
    let s = u64::from_str(parts[2]).unwrap_or(0);
    let ms = u64::from_str(parts[3]).unwrap_or(0);

    (h * 3600 + m * 60 + s) * 1000 + ms
}

async fn vtt_serialize(vtt_path: &Path) -> io::Result<Vec<Subtitle>> {
    let file = File::open(vtt_path).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut subtitles = vec![];
    let mut time_range: Option<(String, String)> = None;

    while let Some(line) = lines.next_line().await? {
        if let Some((s, e)) = time_range.take() {
            let subtitle = Subtitle::new(&s, &e, &line);

            subtitles.push(subtitle);
        }

        time_range = line
            .split_once(" --> ")
            .map(|(s, e)| (s.to_string(), e.to_string()));
    }

    Ok(subtitles)
}

fn split_long_subtitle(subtitle: &Subtitle) -> Vec<Subtitle> {
    let total_duration = subtitle.end as f64 - subtitle.start as f64;
    let words: Vec<&str> = subtitle.text.split_whitespace().collect();
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();

    for word in words {
        if current_chunk.len() + word.len() + 1 > MAX_LEN {
            if let Some(comma_pos) = current_chunk[..].rfind([',', '.', '?', '!', ':', ';']) {
                let (before, after) = current_chunk.split_at(comma_pos + 1);
                chunks.push(before.trim().to_string());
                current_chunk = after.trim().to_string();
            } else {
                chunks.push(current_chunk.trim().to_string());
                current_chunk = String::new();
            }
        }

        if !current_chunk.is_empty() {
            current_chunk.push(' ');
        }
        current_chunk.push_str(word);
    }

    // Add the last segment
    if !current_chunk.is_empty() {
        chunks.push(current_chunk.trim().to_string());
    }

    // Total character count for time distribution
    let total_chars: usize = chunks.iter().map(String::len).sum();

    let mut result = Vec::new();
    let mut current_start = subtitle.start as f64;

    for chunk in chunks {
        let char_count = chunk.len();
        let duration = total_duration * (char_count as f64 / total_chars as f64);
        let end = current_start + duration;

        result.push(Subtitle {
            start: current_start.round() as u64,
            end: end.round() as u64,
            text: chunk,
        });

        current_start = end;
    }

    result
}

fn contains_foreign_script(text: &str) -> Option<char> {
    text.chars().find(|&c| {
        matches!(
            c.script(),
            Script::Arabic
                | Script::Han
                | Script::Hiragana
                | Script::Katakana
                | Script::Hangul
                | Script::Cyrillic
                | Script::Greek
                | Script::Hebrew
                | Script::Devanagari
        )
    })
}

fn process_vtt(subtitles: Vec<Subtitle>, duration: u64, lang: &str) -> Vec<Subtitle> {
    let mut processed = Vec::new();
    let mut foreign_chars = Vec::new();
    let mut prev: Option<Subtitle> = None;

    for current in subtitles {
        if let Some(prev_sub) = prev.take() {
            let prev_text = prev_sub.text.trim();
            let curr_text = current.text.trim();

            if prev_text == curr_text {
                let total_span = current.start.saturating_sub(prev_sub.end);
                if total_span <= MAX_DUPLICATE_DURATION_MS {
                    let merged = Subtitle {
                        start: prev_sub.start,
                        end: current.end,
                        text: curr_text.to_string(),
                    };
                    prev = Some(merged);
                    continue;
                }
                processed.push(prev_sub);
            } else if prev_sub.text.len() + current.text.len() < 121
                && !prev_text.ends_with('.')
                && !prev_text.ends_with('?')
                && !prev_text.ends_with('!')
            {
                let merged = Subtitle {
                    start: prev_sub.start,
                    end: current.end,
                    text: format!("{prev_text} {curr_text}"),
                };
                prev = Some(merged);
                continue;
            } else {
                processed.push(prev_sub);
            }
        }

        if current.text.len() > MAX_LEN {
            let split_subtitles = split_long_subtitle(&current);
            processed.extend(split_subtitles);
            continue;
        }

        if !matches!(
            lang,
            "ar" | "fa" | "ur" | "zh" | "ja" | "ko" | "ru" | "uk" | "he" | "hi"
        ) && let Some(c) = contains_foreign_script(&current.text)
        {
            foreign_chars.push(c);
        }

        prev = Some(current);
    }

    if let Some(mut last) = prev {
        if last.end > duration {
            let new_end = (duration - 300).max(last.start + 500); // min. 500ms length
            last.end = new_end;
        }

        processed.push(last);
    }

    if !foreign_chars.is_empty() {
        error!("Foreign characters found: {foreign_chars:?}");
    }

    processed
}

async fn write_vtt<P: AsRef<Path>>(path: P, subtitles: &[Subtitle]) -> io::Result<()> {
    let mut file = File::create(path).await?;

    file.write_all(b"WEBVTT\n\n").await?;

    for subtitle in subtitles {
        file.write_all(format!("{subtitle}\n").as_bytes()).await?;

        if Some(subtitle) != subtitles.last() {
            file.write_all(b"\n").await?;
        }
    }

    Ok(())
}

pub async fn optimize_vtt(
    in_path: &Path,
    out_path: &Path,
    duration: u64,
    lang: &str,
) -> io::Result<()> {
    let vtt_list = vtt_serialize(in_path).await?;
    let new_vtt_list = process_vtt(vtt_list.clone(), duration, lang);

    if new_vtt_list != vtt_list {
        warn!("Optimized: {out_path:?}");
    }

    write_vtt(out_path, &new_vtt_list).await?;

    let vtt_duration = new_vtt_list.last().cloned().unwrap_or_default().end;

    if duration > 0 && vtt_duration > duration {
        error!(
            "Video with {} is shorter then Webvtt with {} length.",
            format_timestamp(duration),
            format_timestamp(vtt_duration)
        );
    }

    Ok(())
}
