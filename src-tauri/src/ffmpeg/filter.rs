use std::{
    fmt,
    path::{Path, PathBuf},
};

// use log::*;
use regex::Regex;
use serde_json::Value;
use tokio::fs;

use super::{analyze::Lufs, prepare_path, probe::MediaProbe};
use crate::{
    Config, Preset, Task, Template,
    utils::{IMAGE_EXTENSIONS, find_audio, is_close, time_to_sec},
    vec_strings,
};

#[derive(Clone, Debug, Copy, Eq, PartialEq)]
pub enum FilterType {
    Audio,
    Video,
}

impl fmt::Display for FilterType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            FilterType::Audio => write!(f, "a"),
            FilterType::Video => write!(f, "v"),
        }
    }
}

use FilterType::*;

#[derive(Clone, Default, Debug)]
pub struct Filters {
    hw_context: bool,
    pub audio_chain: String,
    pub video_chain: String,
    pub output_chain: Vec<String>,
    pub audio_map: Vec<String>,
    pub video_map: Vec<String>,
    audio_tracks: i32,
    audio_position: i32,
    video_position: i32,
    audio_last: i32,
    video_last: i32,
}

impl Filters {
    pub fn new(hw_context: bool) -> Self {
        Self {
            hw_context,
            audio_chain: String::new(),
            video_chain: String::new(),
            output_chain: vec![],
            audio_map: vec![],
            video_map: vec![],
            audio_tracks: 1,
            audio_position: -1,
            video_position: -1,
            audio_last: -1,
            video_last: -1,
        }
    }

    pub fn add_filter(&mut self, filter: &str, track_nr: i32, filter_type: FilterType) {
        let (map, chain, position, last) = match filter_type {
            Audio => (
                &mut self.audio_map,
                &mut self.audio_chain,
                self.audio_position,
                &mut self.audio_last,
            ),
            Video => (
                &mut self.video_map,
                &mut self.video_chain,
                self.video_position,
                &mut self.video_last,
            ),
        };

        if *last != track_nr {
            // start new filter chain
            let mut selector = String::new();
            let mut sep = String::new();
            if !chain.is_empty() {
                selector = format!("[{filter_type}out{last}]");
                sep = ";".to_string();
            }

            chain.push_str(&selector);

            if filter.starts_with("aevalsrc") || filter.starts_with("movie") {
                chain.push_str(&format!("{sep}{filter}"));
            } else {
                let hw_dl = if self.hw_context && filter_type == Video {
                    "hwdownload,format=nv12,"
                } else {
                    ""
                };

                chain.push_str(&format!(
                    // build audio/video selector like [0:a:0]
                    "{sep}[{position}:{filter_type}:{track_nr}]{hw_dl}{filter}",
                ));
            }

            let m = format!("[{filter_type}out{track_nr}]");
            map.push(m.clone());
            *last = track_nr;
        } else if filter.starts_with(';') || filter.starts_with('[') {
            chain.push_str(filter);
        } else {
            chain.push_str(&format!(",{filter}"));
        }
    }

    pub fn cmd(&mut self) -> Vec<String> {
        if !self.output_chain.is_empty() {
            return self.output_chain.clone();
        }

        let mut v_chain = self.video_chain.clone();
        let mut a_chain = self.audio_chain.clone();

        if self.video_last >= 0 && !v_chain.ends_with(']') {
            v_chain.push_str(&format!("[vout{}]", self.video_last));
        }

        if self.audio_last >= 0 && !a_chain.ends_with(']') {
            a_chain.push_str(&format!("[aout{}]", self.audio_last));
        }

        let mut f_chain = v_chain;
        let mut cmd = vec![];

        if !a_chain.is_empty() && self.audio_position > -1 {
            if !f_chain.is_empty() {
                f_chain.push(';');
            }

            f_chain.push_str(&a_chain);
        }

        if f_chain.contains("concat") {
            f_chain = move_concat(&f_chain);
        }

        if !f_chain.is_empty() {
            cmd.push("-filter_complex".to_string());
            cmd.push(f_chain);
        }

        cmd
    }

    pub fn map_video(&mut self) -> Vec<String> {
        let v_map = "0:v".to_string();
        let mut o_map = vec_strings!["-map"];

        o_map.extend(self.video_map.clone());

        if o_map.len() == 1 {
            o_map.push(v_map);
        }

        o_map
    }

    pub fn map_audio(&mut self) -> Vec<String> {
        let mut o_map = vec_strings!["-map"];
        o_map.extend(self.audio_map.clone());

        if self.audio_last == -1 && self.audio_position > -1 {
            for i in 0..self.audio_tracks {
                let a_map = format!("{}:a:{i}", self.audio_position);

                if !o_map.contains(&a_map) {
                    if i > 0 {
                        o_map.push("-map".to_string());
                    }
                    o_map.push(a_map);
                };
            }
        }

        o_map
    }
}

#[derive(Clone, Default, Debug)]
struct TargetSpec {
    width: i64,
    height: i64,
    aspect: f64,
}

impl TargetSpec {
    fn new(task: &Task, preset: &Preset) -> Self {
        let mut aspect = 0.0;
        let mut resolution = (-1, -1);

        fn resolution_from_map(
            map: &serde_json::Map<String, Value>,
            key: &str,
        ) -> Option<(i64, i64)> {
            if let Some(Value::String(s)) = map.get(key) {
                let (w, h) = s
                    .split_once(':')
                    .or_else(|| s.split_once('x'))
                    .unwrap_or(("-1", "-1"));
                let w = w.parse::<i64>().unwrap_or_default();
                let h = h.parse::<i64>().unwrap_or_default();
                if w > 0 && h > 0 {
                    return Some((w, h));
                }
            }
            None
        }

        if let Value::Object(map) = &preset.filter_video {
            if let Some(res) = resolution_from_map(map, "scale") {
                resolution = res;
            }

            if let Some(Value::String(aspect_str)) = map.get("setdar")
                && let Some(aspect_value) = aspect_str
                    .split_once('=')
                    .map(|s| s.1)
                    .and_then(|n| n.parse::<f64>().ok())
            {
                aspect = aspect_value;
            }
        }

        if let Value::Object(map) = &preset.video {
            if let Some(res) = resolution_from_map(map, "-s") {
                resolution = res;
            }

            if let Some(Value::String(aspect_str)) = map.get("-aspect")
                && let Some((w, h)) = aspect_str
                    .split_once(':')
                    .and_then(|(n1, n2)| Some((n1.parse::<f64>().ok()?, n2.parse::<f64>().ok()?)))
            {
                aspect = w / h;
            }
        }

        if (resolution.0 <= 0 || resolution.1 <= 0)
            && let Some(w) = task.probe.video.first().and_then(|v| v.width)
            && let Some(h) = task.probe.video.first().and_then(|v| v.height)
        {
            resolution = (w, h);
        }

        if aspect == 0.0 {
            aspect = resolution.0 as f64 / resolution.1 as f64;
        }

        Self {
            width: resolution.0,
            height: resolution.1,
            aspect,
        }
    }
}

fn hw_upload_str(input: &Option<String>) -> String {
    if input.as_ref().is_some_and(|i| i.contains("cuda")) {
        return "hwupload_cuda".to_string();
    }

    "hwupload".to_string()
}

fn move_concat(input: &str) -> String {
    let re = Regex::new(r";(\[[a-z0-9_]+\])+concat=n=\d+:v=\d+:a=\d+(\[[a-z0-9_]+\])+").unwrap();

    if let Some(captures) = re.find(input) {
        let matched = captures.as_str();
        let trimmed = input.replacen(matched, "", 1);
        format!("{trimmed}{matched}")
    } else {
        input.to_string()
    }
}

fn pad(probe: &MediaProbe, target_spec: &TargetSpec) -> String {
    let v_stream = probe.video.first();
    let source_aspect = probe.aspect();
    let aspect_mismatch = !is_close(source_aspect, target_spec.aspect, 0.03);
    let mut pad = String::new();

    // Only proceed if a video stream exists and the aspect ratio doesn't match
    if let (Some(v_stream), true) = (v_stream, aspect_mismatch)
        && let (Some(w), Some(h)) = (v_stream.width, v_stream.height)
    {
        let scale = if w > target_spec.width && source_aspect > target_spec.aspect {
            format!("scale={}:-1,", target_spec.width)
        } else if h > target_spec.height && source_aspect < target_spec.aspect {
            format!("scale=-1:{},", target_spec.height)
        } else {
            String::new()
        };

        pad = format!(
            "{scale}pad=max(iw\\,ih*{0:.3}):(ow/{0:.3}):((ow-iw)/2):((oh-ih)/2)",
            target_spec.aspect
        );
    }

    pad
}

fn fade(task: &Task, chain: &mut Filters, typ: FilterType) {
    if task.fade {
        let p = if typ == Audio { "a" } else { "" };

        let st = if task.out > 0.0 {
            task.out - task.r#in - 1.0
        } else {
            task.probe.format.duration.unwrap_or(task.out - task.r#in) - 1.0
        };

        chain.add_filter(&format!("{p}fade=in:d=0.5"), 0, typ);
        chain.add_filter(&format!("{p}fade=out:st={st}:d=1.0"), 0, typ);
    }
}

async fn lower_third(
    path: &str,
    task_probe: &MediaProbe,
    template: &Template,
    preset: &Preset,
    target_spec: &TargetSpec,
    selector: &str,
) -> (String, String) {
    let path = Path::new(path).parent().unwrap();
    let mut filter = String::new();
    let mut base_layer = selector.to_string();
    let mut base_index = 0;
    let mut layer_index = 0;

    // Regex to detect four digits before the file extension, e.g., image_0000.png
    let re = Regex::new(r"(.*?)(\d{4})\.(\w+)$").unwrap();

    for lt in &template.lower_thirds {
        let p = PathBuf::from(&lt.path.replace("\\", "/"));
        let src = if p.is_relative() { path.join(p) } else { p };
        let extension = src
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();

        let mut layer_src = src.clone();

        let mut is_image = IMAGE_EXTENSIONS.contains(&extension.as_str());
        let mut use_sequence = false;

        if is_image
            && let Some(caps) = re.captures(&src.file_name().unwrap_or_default().to_string_lossy())
        {
            let prefix = caps.get(1).unwrap().as_str();
            let suffix = caps.get(3).unwrap().as_str();

            let fallback_dir = PathBuf::from(".");
            let dir = src.parent().unwrap_or(fallback_dir.as_path());
            let mut files = fs::read_dir(dir).await.unwrap();

            // Check if more files with the same prefix and sequential numbers exist
            let mut count = 0;
            while let Ok(Some(entry)) = files.next_entry().await {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if file_name.starts_with(prefix)
                    && file_name.ends_with(&format!(".{}", suffix))
                    && Regex::new(&format!(
                        r"{}(\d{{4}})\.{}",
                        regex::escape(prefix),
                        regex::escape(suffix)
                    ))
                    .unwrap()
                    .is_match(&file_name)
                {
                    count += 1;
                    if count >= 2 {
                        use_sequence = true;
                        break;
                    }
                }
            }

            if use_sequence {
                let new_filename = format!("{}%04d.{}", prefix, suffix);
                layer_src = dir.join(new_filename);
                is_image = false;
            }
        }

        let mut layer_base = format!(
            "movie='{}'",
            prepare_path(layer_src.to_string_lossy().to_string())
        );

        if lt.duration > 0.0 && is_image {
            layer_base.push_str(&format!(
                ":loop=0,setpts=N/(FRAME_RATE*TB),trim=duration={},fade=in:d=0.5:alpha=1,fade=out:st={}:d=0.5:alpha=1",
                lt.duration, lt.duration - 0.5
            ));
        }

        if let Some(scale) = preset.filter_video.get("scale") {
            if let Value::String(s) = scale {
                layer_base.push_str(&format!(",scale={s},setdar=dar={:.3}", target_spec.aspect));
            }
        } else if let Some(w) = task_probe.video[0].width
            && let Some(h) = task_probe.video[0].height
        {
            layer_base.push_str(&format!(
                ",scale={w}:{h},setdar=dar={:.3}",
                target_spec.aspect
            ));
        }

        filter.push(';');

        for (i, position) in lt.position.iter().enumerate() {
            let pos = time_to_sec(position);
            let mut layer = format!("{}{layer_base}", if i > 0 { ";" } else { "" });
            layer.push_str(&format!(",setpts=PTS+{pos}/TB[layer_{layer_index}];"));

            filter.push_str(&layer);
            filter.push_str(&format!(
                "{base_layer}[layer_{layer_index}]overlay=repeatlast=0[base_{base_index}]",
            ));

            base_layer = format!("[base_{base_index}]");
            base_index += 1;
            layer_index += 1;
        }
    }

    (filter, base_layer)
}

async fn intro_outro(
    config: &Config,
    path: &str,
    task_probe: &MediaProbe,
    template: &Template,
    preset: &Preset,
    target_spec: &TargetSpec,
) -> (String, String) {
    let path = Path::new(path).parent().unwrap();
    let mut intro = String::new();
    let mut outro = String::new();
    let mut intro_probe = None;
    let mut outro_probe = None;

    if let Some(i) = &template.intro {
        let p = PathBuf::from(i.replace("\\", "/"));
        let src = if p.is_relative() { path.join(p) } else { p };
        let probe = MediaProbe::new(config, &src).await;

        intro = format!(
            "movie='{}'",
            prepare_path(src.to_string_lossy().to_string())
        );

        if template.intro_duration > 0.0 {
            intro.push_str(&format!(
                ":loop=0,setpts=N/(FRAME_RATE*TB),trim=duration={},fade=in:d=0.5,fade=out:st={}:d=0.5",
                template.intro_duration, template.intro_duration - 0.5
            ));
        }

        if let Ok(probe) = probe {
            intro_probe = Some(probe.clone());

            if !probe.audio.is_empty() {
                intro.push_str(":s=dv+da[intro_v][intro_aout]");
            } else if let Some(audio_src) = find_audio(&src).await {
                intro.push_str(&format!(
                    "[intro_v];amovie='{}'[intro_aout]",
                    prepare_path(audio_src.to_string_lossy().to_string())
                ));
            } else {
                intro.push_str(&format!(
                    "[intro_v];aevalsrc=0:channel_layout=stereo:duration={}:sample_rate=48000[intro_aout]",
                    probe
                        .video
                        .first()
                        .and_then(|v| v.duration)
                        .filter(|&d| d > 0.0)
                        .unwrap_or(template.intro_duration)
                ));
            }
        }
    }

    if let Some(o) = &template.outro {
        let p = PathBuf::from(o.replace("\\", "/"));
        let src = if p.is_relative() { path.join(p) } else { p };
        let probe = MediaProbe::new(config, &src).await;

        outro = format!(
            "movie='{}'",
            prepare_path(src.to_string_lossy().to_string())
        );

        if template.outro_duration > 0.0 {
            outro.push_str(&format!(
                ":loop=0,setpts=N/(FRAME_RATE*TB),trim=duration={},fade=in:d=0.5,fade=out:st={}:d=0.5",
                template.outro_duration, template.outro_duration - 0.5
            ));
        }

        if let Ok(probe) = probe {
            outro_probe = Some(probe.clone());

            if !probe.audio.is_empty() {
                outro.push_str(":s=dv+da[outro_v][outro_aout]");
            } else if let Some(audio_src) = find_audio(&src).await {
                outro.push_str(&format!(
                    "[outro_v];amovie='{}'[outro_aout]",
                    prepare_path(audio_src.to_string_lossy().to_string())
                ));
            } else {
                outro.push_str(&format!(
                    "[outro_v];aevalsrc=0:channel_layout=stereo:duration={}:sample_rate=48000[outro_aout]",
                    probe
                        .video
                        .first()
                        .and_then(|v| v.duration)
                        .filter(|&d| d > 0.0)
                        .unwrap_or(template.outro_duration)
                ));
            }
        }
    }

    let mut sf = "null".to_string();

    if let Some(scale) = preset.filter_video.get("scale") {
        if let Value::String(s) = scale
            && let Some(prob) = intro_probe.or(outro_probe)
        {
            let mut pad_f = pad(&prob, target_spec);

            if !pad_f.is_empty() {
                pad_f.push(',');
            }

            sf = format!("{pad_f}scale={s}");

            if target_spec.aspect > 0.0
                && target_spec.width > 0
                && is_close(
                    target_spec.width as f64 / target_spec.height as f64,
                    target_spec.aspect,
                    0.3,
                )
            {
                sf.push_str(",setsar=sar=1/1");
            }
        }
    } else if let Some(w) = task_probe.video.first().and_then(|v| v.width)
        && let Some(h) = task_probe.video.first().and_then(|v| v.height)
        && let Some(prob) = intro_probe.or(outro_probe)
    {
        let mut pad_f = pad(&prob, target_spec);

        if !pad_f.is_empty() {
            pad_f.push(',');
        }

        sf = format!("{pad_f}scale={w}:{h}");

        if target_spec.aspect > 0.0
            && target_spec.width > 0
            && is_close(
                target_spec.width as f64 / target_spec.height as f64,
                target_spec.aspect,
                0.3,
            )
        {
            sf.push_str(",setsar=sar=1/1");
        }
    }

    if !intro.is_empty() {
        intro.push_str(&format!(";[intro_v]{sf}[intro_vout]"));
    }

    if !outro.is_empty() {
        outro.push_str(&format!(";[outro_v]{sf}[outro_vout]"));
    }

    (intro, outro)
}

fn map_filter(value: &Value, target_spec: &TargetSpec) -> String {
    let mut filters = vec![];

    if let Value::Object(map) = value {
        for (key, val) in map {
            let v = match val {
                Value::String(s) => s.clone(),
                _ => val.to_string(),
            };

            let mut filter = format!("{key}={v}");

            if key == "scale" && target_spec.aspect > 0.0 {
                filter.push_str(&format!(",setdar=dar={:.3}", target_spec.aspect));

                if is_close(
                    target_spec.width as f64 / target_spec.height as f64,
                    target_spec.aspect,
                    0.3,
                ) {
                    filter.push_str(",setsar=sar=1/1");
                }
            }

            filters.push(filter);
        }
    }

    filters.join(",")
}

fn has_codec_copy(args: &Value, typ: FilterType) -> bool {
    let s = match typ {
        Video => 'v',
        Audio => 'a',
    };

    let alias_1 = format!("-codec:{}", s);
    let alias_2 = format!("-{}codec", s);
    let alias_3 = format!("-c:{}", s);
    let alias_4 = "-c";

    if let Value::Object(map) = args {
        for (key, value) in map {
            if (key == &alias_1 || key == &alias_2 || key == &alias_3 || key == alias_4)
                && value == "copy"
            {
                return true;
            }
        }
    }

    false
}

fn loudnorm(lufs: &Lufs) -> String {
    format!(
        "loudnorm=I={}:LRA={}:TP={}:measured_I={}:measured_LRA={}:measured_TP={}:measured_thresh={}:offset={}:linear=true:print_format=summary",
        lufs.target_i,
        lufs.target_lra,
        lufs.target_tp,
        lufs.input_i,
        lufs.input_lra,
        lufs.input_tp,
        lufs.input_thresh,
        lufs.target_offset
    )
}

pub async fn filter_chain(
    config: &Config,
    task: &Task,
    preset: &Preset,
    lufs: &Lufs,
    has_audio: bool,
    has_video: bool,
    audio_pos: i32,
) -> Filters {
    let hw_context = preset.input.as_ref().is_some_and(|i| i.contains("-hw"));
    let mut hw_up = None;
    let mut chain = Filters::new(hw_context);

    if hw_context {
        hw_up = Some(hw_upload_str(&preset.input));
    }

    if has_audio && !has_codec_copy(&preset.audio, Audio) {
        chain.audio_position = audio_pos;
        let filter_audio = map_filter(&preset.filter_audio, &TargetSpec::default());

        if !filter_audio.is_empty() {
            chain.add_filter(&filter_audio, 0, Audio);
        }

        if task.lufs {
            chain.add_filter(&loudnorm(lufs), 0, Audio);
        }

        fade(task, &mut chain, Audio);
    }

    if has_video && !has_codec_copy(&preset.video, Video) {
        chain.video_position = 0;

        let target_spec = TargetSpec::new(task, preset);
        let pad_f = pad(&task.probe, &target_spec);

        if !pad_f.is_empty() {
            chain.add_filter(&pad_f, 0, Video);
        }

        let filter_video = map_filter(&preset.filter_video, &target_spec);

        if filter_video.is_empty() {
            chain.add_filter("null", 0, Video);
        } else {
            chain.add_filter(&filter_video, 0, Video);
        }

        fade(task, &mut chain, Video);

        if let Some(template) = &task.template {
            let mut c_count = 1;
            let mut selectors = vec![];

            let (f, mut main_selector) = lower_third(
                &task.path,
                &task.probe,
                template,
                preset,
                &target_spec,
                "[main_v]",
            )
            .await;
            let (i, o) = intro_outro(
                config,
                &task.path,
                &task.probe,
                template,
                preset,
                &target_spec,
            )
            .await;

            if !f.is_empty() {
                chain.add_filter("[main_v]", 0, Video);
                chain.add_filter(&f, 0, Video);
            } else if filter_video.is_empty() {
                chain.add_filter("null[main_vout]", 0, Video);
                main_selector = "[main_vout]".to_string();
            } else {
                chain.add_filter("[main_vout]", 0, Video);
                main_selector = "[main_vout]".to_string();
            };

            if chain.audio_chain.is_empty() {
                chain.add_filter("anull[main_aout]", 0, Audio);
            } else {
                chain.add_filter("[main_aout]", 0, Audio);
            }

            if !i.is_empty() {
                c_count += 1;
                selectors.extend(["[intro_vout]", "[intro_aout]"]);
                chain.add_filter(&format!(";{i}"), 0, Video);
            }

            selectors.push(&main_selector);
            selectors.push("[main_aout]");

            if !o.is_empty() {
                c_count += 1;
                selectors.extend(["[outro_vout]", "[outro_aout]"]);
                chain.add_filter(&format!(";{o}"), 0, Video);
            }

            if let Some(up) = hw_up {
                chain.add_filter(
                    &format!(
                        ";{}concat=n={c_count}:v=1:a=1[vo][aout0];[vo]{up}[vout0]",
                        selectors.join("")
                    ),
                    0,
                    Video,
                );
            } else {
                chain.add_filter(
                    &format!(
                        ";{}concat=n={c_count}:v=1:a=1[vout0][aout0]",
                        selectors.join("")
                    ),
                    0,
                    Video,
                );
            };
        } else if let Some(up) = hw_up {
            chain.add_filter(&up, 0, Video);
        }
    }

    chain
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        sync::{Arc, atomic::AtomicBool},
    };

    use path_clean::PathClean;
    use serde_json::Map;

    use super::*;

    use crate::{MediaProbe, utils::template::LowerThird};

    fn preset() -> Preset {
        Preset {
            name: "adtv".to_string(),
            title: "ADtv".to_string(),
            tooltip: "Encoding settings for ADTV (x265)".to_string(),
            input: None,
            lufs: None,
            filter_video: {
                let mut map = Map::new();
                map.insert("scale".to_string(), Value::String("1920:1080".to_string()));
                Value::Object(map)
            },
            filter_audio: {
                let map: Map<String, Value> = Map::new();

                Value::Object(map)
            },
            video: {
                let mut map = Map::new();
                map.insert("-pix_fmt".to_string(), Value::String("yuv420p".to_string()));
                map.insert("-r".to_string(), Value::Number(25.into()));
                // map.insert("-aspect".to_string(), Value::String("16:9".to_string()));
                map.insert("-c:v".to_string(), Value::String("libx265".to_string()));
                map.insert("-preset".to_string(), Value::String("medium".to_string()));
                map.insert("-crf".to_string(), Value::Number(22.into()));
                map.insert("-tag:v".to_string(), Value::String("hvc1".to_string()));

                Value::Object(map)
            },
            audio: {
                let mut map = Map::new();
                map.insert("-c:a".to_string(), Value::String("libfdk_aac".to_string()));
                map.insert("-b:a".to_string(), Value::String("192k".to_string()));
                map.insert("-ar".to_string(), Value::String("48k".to_string()));

                Value::Object(map)
            },
            container_video: Some("mp4".to_string()),
            container_audio: None,
            finished: Arc::new(AtomicBool::new(false)),
            output_path: None,
        }
    }

    fn template() -> Template {
        let intro = env::current_dir()
            .unwrap()
            .join("../test/assets/media/colors_1.mp4")
            .clean();
        let outro = env::current_dir()
            .unwrap()
            .join("../test/assets/media/no_audio.mp4")
            .clean();

        Template {
            intro: Some(intro.to_string_lossy().to_string()),
            intro_duration: 0.0,
            outro: Some(outro.to_string_lossy().to_string()),
            outro_duration: 0.0,
            lower_thirds: vec![
                LowerThird {
                    path: "lower_third_1.png".to_string(),
                    duration: 5.0,
                    position: vec_strings!["00:00:01:880", "00:00:8:880"],
                },
                LowerThird {
                    path: "lower_third_2.mov".to_string(),
                    duration: 0.0,
                    position: vec_strings!["00:00:17.300"],
                },
            ],
        }
    }

    async fn task() -> Task {
        let path = env::current_dir()
            .unwrap()
            .join("../test/assets/media/with_audio.mp4")
            .clean();
        let config = Config::default();

        Task {
            id: "test-task".to_string(),
            path: path.to_string_lossy().to_string(),
            url: None,
            presets: vec![preset()],
            template: Some(template()),
            fade: true,
            out: 0.0,
            transcript: None,
            r#in: 0.0,
            probe: MediaProbe::new(&config, &path.to_string_lossy().to_string())
                .await
                .unwrap(),
            publish: None,
            active: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
            lufs: false,
            target: None,
            target_subfolder: false,
        }
    }

    #[tokio::test]
    async fn concat() {
        let task = task().await;
        let config = Config::default();

        let mut filter =
            filter_chain(&config, &task, &preset(), &Lufs::default(), true, true, 0).await;
        let cmd = filter.cmd();
        let mapping_video = filter.map_video();
        let mapping_audio = filter.map_audio();

        println!("{cmd:?}");
        println!("{mapping_video:?}");
        println!("{mapping_audio:?}");
    }

    #[tokio::test]
    async fn no_template() {
        let mut task = task().await;
        task.template = None;
        let config = Config::default();

        let mut filter =
            filter_chain(&config, &task, &preset(), &Lufs::default(), true, true, 0).await;
        let cmd = filter.cmd();
        let mapping_video = filter.map_video();
        let mapping_audio = filter.map_audio();

        println!("{cmd:?}");
        println!("{mapping_video:?}");
        println!("{mapping_audio:?}");
    }

    #[tokio::test]
    async fn resolution_aspect() {
        let task = task().await;
        let res_asp = TargetSpec::new(&task, &task.presets[0]);

        println!("{res_asp:?}");
    }
}
