use std::error::Error;
use std::io::{self, BufRead, IsTerminal, Write};

pub(super) fn confirm(
    prompt: &str,
    non_interactive_hint: &str,
    yes: bool,
) -> Result<bool, Box<dyn Error>> {
    if yes {
        return Ok(true);
    }

    let stdin = io::stdin();
    let interactive = stdin.is_terminal();
    let mut input = stdin.lock();
    let mut output = io::stderr().lock();

    confirm_with(
        prompt,
        non_interactive_hint,
        yes,
        interactive,
        &mut input,
        &mut output,
    )
}

fn confirm_with(
    prompt: &str,
    non_interactive_hint: &str,
    yes: bool,
    interactive: bool,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<bool, Box<dyn Error>> {
    if yes {
        return Ok(true);
    }
    if !interactive {
        return Err(non_interactive_hint.into());
    }

    write!(output, "{prompt} [y/N] ")?;
    output.flush()?;

    let mut answer = String::new();
    input.read_line(&mut answer)?;

    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

#[cfg(test)]
mod tests {
    use super::confirm_with;
    use std::io::Cursor;

    #[test]
    fn confirmation_accepts_only_explicit_yes_answers() {
        for (answer, expected) in [
            ("y\n", true),
            ("YES\n", true),
            ("n\n", false),
            ("\n", false),
        ] {
            let mut input = Cursor::new(answer.as_bytes());
            let mut output = Vec::new();

            let confirmed = confirm_with(
                "Apply this plan?",
                "use --yes",
                false,
                true,
                &mut input,
                &mut output,
            )
            .expect("read confirmation");

            assert_eq!(confirmed, expected);
            assert_eq!(output, b"Apply this plan? [y/N] ");
        }
    }

    #[test]
    fn non_interactive_confirmation_refuses_without_reading_or_prompting() {
        let mut input = Cursor::new(b"y\n");
        let mut output = Vec::new();

        let error = confirm_with(
            "Apply this plan?",
            "use --yes",
            false,
            false,
            &mut input,
            &mut output,
        )
        .expect_err("non-interactive confirmation must fail");

        assert_eq!(error.to_string(), "use --yes");
        assert_eq!(input.position(), 0);
        assert!(output.is_empty());
    }

    #[test]
    fn explicit_yes_bypasses_interaction_and_input() {
        let mut input = Cursor::new(b"n\n");
        let mut output = Vec::new();

        assert!(
            confirm_with(
                "Apply this plan?",
                "use --yes",
                true,
                false,
                &mut input,
                &mut output,
            )
            .expect("explicit confirmation")
        );
        assert_eq!(input.position(), 0);
        assert!(output.is_empty());
    }
}
