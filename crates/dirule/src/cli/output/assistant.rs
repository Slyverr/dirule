use super::super::args::ColorChoice;
use super::{Colorizer, color_enabled};
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use termimad::MadSkin;

pub(in crate::cli) fn assistant_notice(
    profile: &str,
    model: &str,
    target: &Path,
    color: ColorChoice,
) -> io::Result<()> {
    let styles = Colorizer::stderr(color);
    let mut stderr = io::stderr().lock();

    writeln!(
        stderr,
        "{} · profile {profile:?} · model {model:?}",
        styles.style("Dirule Ask", "36")
    )?;
    writeln!(stderr, "{} {target:?}", styles.style("Target:", "36"))?;
    writeln!(
        stderr,
        "{} your conversation, target path, rule excerpts, preset names, requested paths, previews, and target history. File contents are not read.",
        styles.style("May send:", "33")
    )?;
    writeln!(
        stderr,
        "{} review provider terms before sharing sensitive names or paths.",
        styles.style("Privacy:", "90")
    )?;
    writeln!(stderr)?;

    Ok(())
}

pub(in crate::cli) fn assistant_response(response: &str, color: ColorChoice) -> io::Result<()> {
    let response = sanitize_model_text(response);
    let stdout = io::stdout();
    let is_terminal = stdout.is_terminal();
    let skin = if color_enabled(color, is_terminal) {
        MadSkin::default()
    } else {
        MadSkin::no_style()
    };
    let mut stdout = stdout.lock();
    let rendered = if is_terminal {
        skin.term_text(&response)
    } else {
        skin.text(&response, None)
    }
    .to_string();

    writeln!(stdout, "{}", Colorizer::stdout(color).style("Dirule", "36"))?;
    write!(stdout, "{rendered}")?;
    if !rendered.ends_with('\n') {
        writeln!(stdout)?;
    }
    writeln!(stdout)?;

    stdout.flush()
}

fn sanitize_model_text(response: &str) -> String {
    let response = strip_ansi_escapes::strip_str(response);
    let mut sanitized = String::with_capacity(response.len());

    for character in response.chars() {
        match character {
            '\t' => sanitized.push_str("    "),
            '\n' => sanitized.push('\n'),
            '\r' => {}
            control if control.is_control() => {}
            character => sanitized.push(character),
        }
    }

    sanitized
}
