use anyhow::Error;
use std::env;
use std::io::{self, IsTerminal, Write};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub enum Tone {
    Success,
    Info,
    Warning,
    Error,
}

pub fn print_stdout(tone: Tone, title: &str, body: &str) {
    let unicode = stdout_supports_unicode();
    let mut stdout = io::stdout();
    let _ = write_panel(&mut stdout, unicode, tone, title, body);
}

pub fn print_stderr(tone: Tone, title: &str, body: &str) {
    let unicode = stderr_supports_unicode();
    let mut stderr = io::stderr();
    let _ = write_panel(&mut stderr, unicode, tone, title, body);
}

pub fn start_progress(text: &str) -> ProgressIndicator {
    ProgressIndicator::start(text)
}

pub fn prompt_confirmation(title: &str, body: &str) -> Result<bool, io::Error> {
    let unicode = stdout_supports_unicode();
    let mut stdout = io::stdout();
    write_panel(&mut stdout, unicode, Tone::Warning, title, body)?;
    let prompt = "  Continue? [y/N]: ";
    write!(stdout, "{prompt}")?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    writeln!(stdout)?;
    let answer = input.trim().to_ascii_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

pub fn prompt_yes_no(
    title: &str,
    body: &str,
    action: &str,
    default_yes: bool,
) -> Result<bool, io::Error> {
    let unicode = stdout_supports_unicode();
    let mut stdout = io::stdout();
    write_panel(&mut stdout, unicode, Tone::Info, title, body)?;
    let prompt = if default_yes {
        format!("  {action}? [Y/n]: ")
    } else {
        format!("  {action}? [y/N]: ")
    };
    write!(stdout, "{prompt}")?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    writeln!(stdout)?;
    let answer = input.trim().to_ascii_lowercase();
    if answer.is_empty() {
        return Ok(default_yes);
    }
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

pub fn prompt_choice(title: &str, body: &str, prompt: &str) -> Result<String, io::Error> {
    let unicode = stdout_supports_unicode();
    let mut stdout = io::stdout();
    write_panel(&mut stdout, unicode, Tone::Warning, title, body)?;
    write!(stdout, "  {prompt}")?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    writeln!(stdout)?;
    Ok(input.trim().to_ascii_lowercase())
}

pub fn prompt_text(title: &str, body: &str, prompt: &str) -> Result<String, io::Error> {
    let unicode = stdout_supports_unicode();
    let mut stdout = io::stdout();
    write_panel(&mut stdout, unicode, Tone::Info, title, body)?;
    write!(stdout, "  {prompt}")?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    writeln!(stdout)?;
    Ok(input.trim().to_string())
}

pub fn error_body(error: &Error) -> String {
    let mut body = String::new();
    body.push_str(&error.to_string());

    let details: Vec<String> = error
        .chain()
        .skip(1)
        .map(ToString::to_string)
        .filter(|line| line != &body)
        .collect();

    if !details.is_empty() {
        body.push_str("\n\nDetails:");
        for detail in details {
            body.push('\n');
            body.push_str(&list_item(&detail));
        }
    }

    body
}

pub fn list_item(text: &str) -> String {
    let bullet = if stdout_supports_unicode() {
        "•"
    } else {
        "-"
    };
    format!("{bullet} {text}")
}

pub fn status_line(tone: Tone, text: &str) -> String {
    let unicode = stdout_supports_unicode();
    let glyph = glyph(tone, unicode);
    format!("{glyph} {text}")
}

fn write_panel<W: Write>(
    writer: &mut W,
    unicode: bool,
    tone: Tone,
    title: &str,
    body: &str,
) -> Result<(), io::Error> {
    writeln!(writer)?;
    writeln!(writer, "{} {}", glyph(tone, unicode), title)?;
    if !body.trim().is_empty() {
        writeln!(writer)?;
        for line in body.lines() {
            if line.is_empty() {
                writeln!(writer)?;
            } else {
                writeln!(writer, "  {line}")?;
            }
        }
    }
    writeln!(writer)?;
    Ok(())
}

fn glyph(tone: Tone, unicode: bool) -> &'static str {
    match (tone, unicode) {
        (Tone::Success, true) => "✓",
        (Tone::Info, true) => "›",
        (Tone::Warning, true) => "⚠",
        (Tone::Error, true) => "✗",
        (Tone::Success, false) => "[ok]",
        (Tone::Info, false) => "[>]",
        (Tone::Warning, false) => "[!]",
        (Tone::Error, false) => "[x]",
    }
}

pub struct ProgressIndicator {
    state: Arc<AtomicU8>,
    thread: Option<thread::JoinHandle<()>>,
    interactive: bool,
    label: String,
    finished: bool,
}

impl ProgressIndicator {
    fn start(text: &str) -> Self {
        let interactive = io::stderr().is_terminal();
        let unicode = stderr_supports_unicode();
        let color = stderr_supports_color();
        let label = text.to_string();

        if !interactive {
            return Self {
                state: Arc::new(AtomicU8::new(0)),
                thread: None,
                interactive,
                label,
                finished: false,
            };
        }

        let state = Arc::new(AtomicU8::new(0));
        let worker_state = Arc::clone(&state);
        let worker_label = label.clone();
        let thread = thread::spawn(move || {
            let frames = spinner_frames(unicode);
            let mut stderr = io::stderr();
            let mut frame = 0usize;

            loop {
                match worker_state.load(Ordering::SeqCst) {
                    0 => {
                        let spinner = colorize(frames[frame % frames.len()], Tone::Info, color);
                        let _ = write!(stderr, "\r\x1b[2K{} {}", spinner, worker_label);
                        let _ = stderr.flush();
                        frame += 1;
                        thread::sleep(Duration::from_millis(80));
                    }
                    1 => {
                        let done = colorize(glyph(Tone::Success, unicode), Tone::Success, color);
                        let _ = writeln!(stderr, "\r\x1b[2K{} {}", done, worker_label);
                        let _ = stderr.flush();
                        break;
                    }
                    2 => {
                        let done = colorize(glyph(Tone::Error, unicode), Tone::Error, color);
                        let _ = writeln!(stderr, "\r\x1b[2K{} {}", done, worker_label);
                        let _ = stderr.flush();
                        break;
                    }
                    _ => break,
                }
            }
        });

        Self {
            state,
            thread: Some(thread),
            interactive,
            label,
            finished: false,
        }
    }

    pub fn success(mut self) {
        self.finish(1);
    }

    pub fn error(mut self) {
        self.finish(2);
    }

    fn finish(&mut self, state: u8) {
        if self.finished {
            return;
        }
        self.finished = true;

        if self.interactive {
            self.state.store(state, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            return;
        }

        let unicode = stderr_supports_unicode();
        let color = stderr_supports_color();
        let tone = if state == 1 {
            Tone::Success
        } else {
            Tone::Error
        };
        let glyph = colorize(glyph(tone, unicode), tone, color);
        let mut stderr = io::stderr();
        let _ = writeln!(stderr, "{} {}", glyph, self.label);
        let _ = stderr.flush();
    }
}

impl Drop for ProgressIndicator {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.finish(2);
    }
}

fn stdout_supports_unicode() -> bool {
    io::stdout().is_terminal() && locale_supports_unicode()
}

fn stderr_supports_unicode() -> bool {
    io::stderr().is_terminal() && locale_supports_unicode()
}

fn stderr_supports_color() -> bool {
    io::stderr().is_terminal() && env::var_os("NO_COLOR").is_none()
}

fn locale_supports_unicode() -> bool {
    let locale = env::var("LC_ALL")
        .ok()
        .or_else(|| env::var("LC_CTYPE").ok())
        .or_else(|| env::var("LANG").ok())
        .unwrap_or_default();

    !matches!(locale.as_str(), "C" | "POSIX")
}

fn spinner_frames(unicode: bool) -> &'static [&'static str] {
    if unicode {
        &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
    } else {
        &["-", "\\", "|", "/"]
    }
}

fn colorize(text: &str, tone: Tone, enabled: bool) -> String {
    if !enabled {
        return text.to_string();
    }

    let code = match tone {
        Tone::Success => 32,
        Tone::Info => 36,
        Tone::Warning => 33,
        Tone::Error => 31,
    };
    format!("\x1b[{code}m{text}\x1b[0m")
}
