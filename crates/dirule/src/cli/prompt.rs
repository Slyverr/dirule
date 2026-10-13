use std::error::Error;
use std::io::{self, IsTerminal, Write};

pub(super) fn confirm(
    prompt: &str,
    non_interactive_hint: &str,
    yes: bool,
) -> Result<bool, Box<dyn Error>> {
    if yes {
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        return Err(non_interactive_hint.into());
    }
    eprint!("{prompt} [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
