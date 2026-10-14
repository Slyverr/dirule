use super::{args, output};
use dirule_core::Transform;
use std::error::Error;
use std::path::{Path, PathBuf};

pub(super) fn sort(
    options: args::SortArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let args::SortArgs {
        selectors,
        by,
        operation,
    } = options;
    let target = target(operation.target.as_deref())?;
    let preview = match by {
        Some(args::SortBy::Extension) => {
            dirule_app::sort_by_extension_preview(&target, operation.recursive)?
        }
        None => {
            let categories = dirule_app::CategoryCatalog::user_if_available()?;
            dirule_app::sort_selectors_preview(
                &target,
                &selectors,
                &categories,
                operation.recursive,
            )?
        }
    };

    execute(preview, operation, color)
}

pub(super) fn rename(
    options: args::RenameArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let target = target(options.operation.target.as_deref())?;
    let transform = Transform::from(options.transform);
    let preview = dirule_app::rename_preview(&target, transform, options.operation.recursive)?;

    execute(preview, options.operation, color)
}

fn target(path: Option<&Path>) -> Result<PathBuf, Box<dyn Error>> {
    path.map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(|| std::env::current_dir().map_err(Into::into))
}

fn execute(
    preview: dirule_app::Preview,
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
    if !apply {
        let target = &preview.target;
        let plan = &preview.plan;
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

    super::apply::execute(
        preview,
        super::apply::ApplyOptions {
            format,
            color,
            yes,
            on_conflict,
            verbose: false,
        },
    )
}
