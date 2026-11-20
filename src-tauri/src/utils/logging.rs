use std::io::{self, Write};

use crate::events::AppEvents;
use flexi_logger::{
    DeferredNow, Level, LogSpecification, Logger, LoggerHandle, writers::LogWriter,
};
use log::*;
use regex::Regex;

use crate::utils::VIDEO_EXTENSIONS;

const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.6f";

#[macro_export]
macro_rules! plain {
    ($($arg:tt)*) => {{
        log::info!(target: "{plain}", "{}", format_args!($($arg)*));
    }};
}

#[derive(Clone)]
pub struct CommandLogger {
    last_level: Level,
}

impl CommandLogger {
    pub fn new() -> Self {
        Self {
            last_level: Level::Info,
        }
    }

    pub fn clean_log(&self, input: &str) -> String {
        let re = Regex::new(r"(?i)\s*\[(info|warning|error|fatal)\] ?").unwrap();

        re.replace_all(input, "").to_string()
    }

    pub fn log(&mut self, prefix: Option<&str>, line: &str) {
        let prefix = match prefix {
            Some(p) => format!("<span class=\"text-base-content/60\">{p} </span>"),
            None => String::new(),
        };

        if line.to_lowercase().contains("error:") {
            error!("{prefix}{}", self.clean_log(line));

            self.last_level = Level::Error;
        } else if line.to_lowercase().contains("[info]") {
            info!("{prefix}{}", self.clean_log(line));

            self.last_level = Level::Info;
        } else if line.to_lowercase().contains("[warning]") {
            warn!("{prefix}{}", self.clean_log(line));

            self.last_level = Level::Warn;
        } else if line.to_lowercase().contains("[error]") || line.to_lowercase().contains("[fatal]")
        {
            error!("{prefix}{}", self.clean_log(line));

            self.last_level = Level::Error;
        } else {
            match self.last_level {
                Level::Info => info!("{prefix}{line}"),
                Level::Warn => warn!("{prefix}{line}"),
                Level::Error => error!("{prefix}{line}"),
                _ => debug!("{prefix}{line}"),
            }
        }
    }
}

pub struct LogPlain {
    pub app: AppEvents,
}

impl LogPlain {
    pub fn new(app: AppEvents) -> Self {
        Self { app }
    }
}

impl LogWriter for LogPlain {
    fn write(&self, _now: &mut DeferredNow, record: &Record<'_>) -> std::io::Result<()> {
        if let Err(e) = self.app.emit("logging", record.args().to_string()) {
            eprint!("{e:?}");
        };

        Ok(())
    }
    fn flush(&self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct LogConsole;

impl LogWriter for LogConsole {
    fn write(&self, now: &mut DeferredNow, record: &Record<'_>) -> std::io::Result<()> {
        console_formatter(&mut std::io::stderr(), now, record, false)?;

        Ok(())
    }
    fn flush(&self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct LogPlainConsole;

impl LogWriter for LogPlainConsole {
    fn write(&self, _now: &mut DeferredNow, record: &Record<'_>) -> std::io::Result<()> {
        writeln!(&mut std::io::stderr(), "{}", record.args())?;

        Ok(())
    }
    fn flush(&self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct LogEmitter {
    pub app: AppEvents,
}

impl LogEmitter {
    pub fn new(app: AppEvents) -> Self {
        Self { app }
    }
}

impl LogWriter for LogEmitter {
    fn write(&self, now: &mut DeferredNow, record: &Record<'_>) -> std::io::Result<()> {
        if let Err(e) = self.app.emit("logging", formatter(now, record)) {
            eprint!("{e:?}");
        };

        Ok(())
    }
    fn flush(&self) -> std::io::Result<()> {
        Ok(())
    }
}

fn format_level(record: &Record) -> String {
    match record.level() {
        Level::Trace => format!(
            "<span class=\"level-trace\">[TRACE]</span> {}:{} {}",
            record.file().unwrap_or_default(),
            record.line().unwrap_or_default(),
            record.args()
        ),
        Level::Debug => format!(
            "<span class=\"level-debug\">[DEBUG]</span> {}",
            record.args()
        ),
        Level::Info => format!(
            "<span class=\"level-info\">[ INFO]</span> {}",
            record.args()
        ),
        Level::Warn => format!(
            "<span class=\"level-warning\">[ WARN]</span> {}",
            record.args()
        ),
        Level::Error => format!(
            "<span class=\"level-error\">[ERROR]</span> {}",
            record.args()
        ),
    }
}

fn console_formatter(
    w: &mut dyn Write,
    now: &mut DeferredNow,
    record: &Record,
    plain: bool,
) -> io::Result<()> {
    let log_line = html_to_ansi(&format_level(record));

    if plain {
        writeln!(w, "{log_line}")
    } else {
        let time = now.format(TIME_FORMAT);

        writeln!(
            w,
            "{} {}",
            html_to_ansi(&format!("<span class=\"log-gray\">[{time}]</span>")),
            log_line
        )
    }
}

fn formatter(now: &mut DeferredNow, record: &Record) -> String {
    let log_line = match record.level() {
        Level::Debug => format!(
            "<span class=\"text-cyan-500\">[DEBUG]</span>{}",
            record.args()
        ),
        Level::Info => format!(
            "<span class=\"text-lime-500\">[ INFO]</span> {}",
            record.args()
        ),
        Level::Warn => format!(
            "<span class=\"text-yellow-500\">[ WARN]</span> {}",
            record.args()
        ),
        Level::Error => format!(
            "<span class=\"text-red-500\">[ERROR]</span> {}",
            record.args()
        ),
        _ => format!(
            "<span class=\"text-base-content/50\">[OTHER]</span> {}",
            record.args()
        ),
    };

    let time = now.format("%H:%M:%S%.6f");

    format!("<span class=\"text-base-content/50\">[{time}]</span> {log_line}")
}

fn html_to_ansi(input: &str) -> String {
    let mut output = input.to_string();

    let replacements = vec![
        (
            r#"<span class="level-trace">([^<]+)</span>"#,
            "\x1b[93m$1\x1b[0m",
        ), // level bright yellow
        (
            r#"<span class="level-debug">([^<]+)</span>"#,
            "\x1b[94m$1\x1b[0m",
        ), // level bright blue
        (
            r#"<span class="level-info">([^<]+)</span>"#,
            "\x1b[92m$1\x1b[0m",
        ), // level green
        (
            r#"<span class="level-warning">([^<]+)</span>"#,
            "\x1b[33m$1\x1b[0m",
        ), // level yellow
        (
            r#"<span class="level-error">([^<]+)</span>"#,
            "\x1b[31m$1\x1b[0m",
        ), // level red
        // text and number formatting
        (
            r#"<span class="log-gray">([^<]+)</span>"#,
            "\x1b[90m$1\x1b[0m",
        ), // bright black
        (
            r#"<span class="text-base-content/60">([^<]+)</span>"#,
            "\x1b[90m$1\x1b[0m",
        ), // bright black
        (
            r#"<span class="log-addr">([^<]+)</span>"#,
            "\x1b[1;35m$1\x1b[0m",
        ), // bold magenta
        (
            r#"<span class="log-cmd">([^<]+)</span>"#,
            "\x1b[94m$1\x1b[0m",
        ), // bright blue
        (
            r#"<span class="log-number">([^<]+)</span>"#,
            "\x1b[33m$1\x1b[0m",
        ), // yellow
    ];

    for (pattern, replacement) in replacements {
        let re = Regex::new(pattern).unwrap();
        output = re.replace_all(&output, replacement).to_string();
    }

    output
}

pub fn init_logging(app: Option<AppEvents>) -> Result<LoggerHandle, io::Error> {
    // Build the initial log specification
    let mut builder = LogSpecification::builder();
    builder
        .default(LevelFilter::Debug)
        .module("log", LevelFilter::Error)
        .module("rpc", LevelFilter::Error)
        .module("tao", LevelFilter::Error)
        .module("tokio", LevelFilter::Error)
        .module("winit", LevelFilter::Error)
        .module("winit-gtk", LevelFilter::Error)
        .module("reqwest", LevelFilter::Error)
        .module("tauri_plugin_http", LevelFilter::Error)
        .module("rustls_platform_verifier", LevelFilter::Error)
        .module("tauri_plugin_updater", LevelFilter::Error);

    let mut logger = Logger::with(builder.build());

    if let Some(a) = app {
        logger = logger
            .log_to_writer(Box::new(LogEmitter::new(a.clone())))
            .add_writer("plain", Box::new(LogPlain::new(a)));
    } else {
        logger = logger
            .log_to_writer(Box::new(LogConsole))
            .add_writer("plain", Box::new(LogPlainConsole));
    }

    logger.start().map_err(|e| io::Error::other(e.to_string()))
}

pub fn log_command(title: &str, prefix: Option<String>, mut cmd: Vec<String>) {
    let max_line_length = 160;
    let mut formatted_cmd = Vec::new();
    let mut quote_next = false;
    let mut current_line = String::new();
    let mut is_first_line = true;
    let mut last_arg = String::new();

    if let Some(pr) = prefix {
        cmd.insert(0, pr);
    }

    debug!("-------------------------------------------------------------------");
    debug!("{title}");

    for (i, arg) in cmd.iter().enumerate() {
        if quote_next
            || (i == cmd.len() - 1)
            || VIDEO_EXTENSIONS.contains(
                &arg.rsplit('.')
                    .next()
                    .unwrap_or_default()
                    .to_lowercase()
                    .as_str(),
            )
        {
            formatted_cmd.push(format!("\"{}\"", arg));
            quote_next = false;
        } else {
            formatted_cmd.push(arg.to_string());
            // TODO: podman use also -i; command after should not be in quotes
            if ["-i", "-filter_complex", "-map", "-metadata"].contains(&arg.as_str()) {
                quote_next = true;
            }
        }
    }

    for arg in formatted_cmd {
        if last_arg == "-filter_complex" {
            if is_first_line {
                debug!("{current_line} \\");
                is_first_line = false;
            } else {
                plain!("{current_line} \\");
            }

            current_line.clear();

            let filter_lines: Vec<&str> = arg.split(';').collect();
            for (i, line) in filter_lines.iter().enumerate() {
                if i == filter_lines.len() - 1 {
                    plain!("{line} \\");
                } else {
                    plain!("{line}; \\");
                }
            }

            last_arg.clear();
        } else if current_line.len() + arg.len() + 1 < max_line_length {
            if !current_line.is_empty() {
                current_line.push(' ');
            }
            current_line.push_str(&arg);
        } else {
            if is_first_line {
                debug!("{current_line} \\");
                is_first_line = false;
            } else {
                plain!("{current_line} \\");
            }
            current_line = arg.clone();
        }

        last_arg = arg.clone();
    }

    if !current_line.is_empty() {
        if is_first_line {
            debug!("{current_line}");
        } else {
            plain!("{current_line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jiff_timestamps_keep_microseconds_in_console_and_gui_logs() {
        let mut now = DeferredNow::new();
        let timestamp = now.format(TIME_FORMAT).to_string();
        assert!(
            Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{6}$")
                .unwrap()
                .is_match(&timestamp)
        );
        let record = Record::builder()
            .args(format_args!("Test message"))
            .level(Level::Info)
            .build();
        let mut console = Vec::new();
        console_formatter(&mut console, &mut now, &record, false).unwrap();
        assert!(String::from_utf8(console).unwrap().contains(&timestamp));

        let time = now.format("%H:%M:%S%.6f").to_string();
        let gui = formatter(&mut now, &record);
        assert!(gui.contains(&format!("[{time}]")));
        assert!(gui.contains("Test message"));
    }
}
