use super::{args, journal, output, prompt};
use dirule_core::Transform;
use dirule_fs::ConflictPolicy;
use std::error::Error;
use std::path::{Path, PathBuf};

pub(super) fn sort(
    options: args::SortArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let args::SortArgs {
        extension,
        by,
        operation,
    } = options;
    let target = target(operation.target.as_deref())?;
    let preview = match (extension, by) {
        (Some(extension), None) => {
            crate::application::sort_extension_preview(&target, &extension, operation.recursive)?
        }
        (None, Some(args::SortBy::Extension)) => {
            crate::application::sort_by_extension_preview(&target, operation.recursive)?
        }
        _ => return Err("choose one extension or a supported --by value".into()),
    };

    execute(preview, operation, color)
}

pub(super) fn rename(
    options: args::RenameArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let target = target(options.operation.target.as_deref())?;
    let transform = Transform::from(options.transform);
    let preview =
        crate::application::rename_preview(&target, transform, options.operation.recursive)?;

    execute(preview, options.operation, color)
}

fn target(path: Option<&Path>) -> Result<PathBuf, Box<dyn Error>> {
    path.map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(|| std::env::current_dir().map_err(Into::into))
}

fn execute(
    preview: crate::application::Preview,
    options: args::QuickOperationArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let args::QuickOperationArgs {
        apply,
        yes,
        on_conflict,
        format,
        ..
    } = options;
    let target = &preview.target;
    let plan = &preview.plan;

    if !apply {
        output::plan(
            target,
            plan,
            format,
            color,
            output::PlanContext::Preview,
            false,
        )?;
        if !plan.is_conflict_free() {
            return Err(format!("plan contains {} conflict(s)", plan.conflicts.len()).into());
        }

        return Ok(());
    }

    if format == args::OutputFormat::Human {
        output::plan(
            target,
            plan,
            format,
            color,
            output::PlanContext::Apply,
            false,
        )?;
    } else if output::json_apply_preview_needed(format, yes, plan) {
        output::plan_to_stderr(target, plan, color, output::PlanContext::Apply, false)?;
    }

    if !plan.is_conflict_free() && on_conflict == args::ConflictAction::Abort {
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
            format,
            color,
        )?;
        return Ok(());
    }
    if !prompt::confirm(
        "Apply this plan?",
        "use --yes to apply without an interactive terminal",
        yes,
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
            format,
            color,
        )?;
        return Ok(());
    }

    let policy = match on_conflict {
        args::ConflictAction::Abort => ConflictPolicy::Abort,
        args::ConflictAction::Skip => ConflictPolicy::Skip,
    };
    let store = journal::store()?;
    let applied = crate::application::apply_authorized(preview, policy, &store)?;
    output::apply_result(
        &applied.target,
        &applied.plan,
        output::ApplySummary {
            status: output::ApplyStatus::Applied,
            moved: applied.report.moved.len(),
            skipped: applied.report.skipped.len(),
            run_id: applied.run_id.as_deref(),
        },
        format,
        color,
    )?;

    Ok(())
}
