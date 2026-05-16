use anyhow::Error;
use std::env;
use std::io::{self, IsTerminal, Write};

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

pub fn prompt_confirmation(title: &str, body: &str) -> Result<bool, io::Error> {
    let unicode = stdout_supports_unicode();
    let mut stdout = io::stdout();
    write_panel(&mut stdout, unicode, Tone::Warning, title, body)?;
    let prompt = if unicode {
        "  Continue? [y/N]: "
    } else {
        "  Continue? [y/N]: "
    };
    write!(stdout, "{prompt}")?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    writeln!(stdout)?;
    let answer = input.trim().to_ascii_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

pub fn prompt_yes_no(title: &str, body: &str, default_yes: bool) -> Result<bool, io::Error> {
    let unicode = stdout_supports_unicode();
    let mut stdout = io::stdout();
    write_panel(&mut stdout, unicode, Tone::Info, title, body)?;
    let prompt = if default_yes {
        "  Continue? [Y/n]: "
    } else {
        "  Continue? [y/N]: "
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

fn stdout_supports_unicode() -> bool {
    io::stdout().is_terminal() && locale_supports_unicode()
}

fn stderr_supports_unicode() -> bool {
    io::stderr().is_terminal() && locale_supports_unicode()
}

fn locale_supports_unicode() -> bool {
    let locale = env::var("LC_ALL")
        .ok()
        .or_else(|| env::var("LC_CTYPE").ok())
        .or_else(|| env::var("LANG").ok())
        .unwrap_or_default();

    !matches!(locale.as_str(), "C" | "POSIX")
}
