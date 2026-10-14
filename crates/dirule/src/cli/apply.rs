use super::{args, journal, output, prompt};
use dirule_fs::ConflictPolicy;
use std::error::Error;

pub(super) struct ApplyOptions {
    pub(super) format: args::OutputFormat,
    pub(super) color: args::ColorChoice,
    pub(super) yes: bool,
    pub(super) on_conflict: args::ConflictAction,
    pub(super) verbose: bool,
}

pub(super) fn execute(
    preview: dirule_app::Preview,
    options: ApplyOptions,
) -> Result<(), Box<dyn Error>> {
    let target = &preview.target;
    let plan = &preview.plan;

    if options.format == args::OutputFormat::Human {
        output::plan(
            target,
            plan,
            options.format,
            options.color,
            output::PlanContext::Apply,
            options.verbose,
        )?;
    } else if output::json_apply_preview_needed(options.format, options.yes, plan) {
        output::plan_to_stderr(
            target,
            plan,
            options.color,
            output::PlanContext::Apply,
            options.verbose,
        )?;
    }

    if !plan.is_conflict_free() && options.on_conflict == args::ConflictAction::Abort {
        return Err(format!("plan contains {} conflict(s)", plan.conflicts.len()).into());
    }
    if !plan.has_changes() {
        output::apply_result(
            target,
            plan,
            output::ApplySummary {
                status: output::ApplyStatus::NoChanges,
                moved: 0,
                skipped: 0,
                run_id: None,
            },
            options.format,
            options.color,
        )?;

        return Ok(());
    }
    if !prompt::confirm(
        "Apply this plan?",
        "use --yes to apply without an interactive terminal",
        options.yes,
    )? {
        output::apply_result(
            target,
            plan,
            output::ApplySummary {
                status: output::ApplyStatus::Cancelled,
                moved: 0,
                skipped: 0,
                run_id: None,
            },
            options.format,
            options.color,
        )?;

        return Ok(());
    }

    let policy = match options.on_conflict {
        args::ConflictAction::Abort => ConflictPolicy::Abort,
        args::ConflictAction::Skip => ConflictPolicy::Skip,
    };
    let store = journal::store()?;
    let applied = dirule_app::apply_authorized(preview, policy, &store)?;
    output::apply_result(
        &applied.target,
        &applied.plan,
        output::ApplySummary {
            status: output::ApplyStatus::Applied,
            moved: applied.report.moved.len(),
            skipped: applied.report.skipped.len(),
            run_id: applied.run_id.as_deref(),
        },
        options.format,
        options.color,
    )?;

    Ok(())
}
