use std::path::Path;

use anyhow::Context;
use glob::glob;

use crate::result::LogIfError;

pub(crate) fn visit_files(
    path: impl AsRef<Path>,
    glob_expr: &str,
    found_err: &mut bool,
    mut f: impl FnMut(&Path, &mut bool) -> anyhow::Result<()>,
) {
    let path = path.as_ref();
    let glob_expr = path.join(glob_expr);
    let glob_expr = glob_expr.to_str().unwrap();
    glob(&glob_expr)
        .context("glob pattern {path:?} is invalid")
        .unwrap()
        .for_each(|path_res| {
            if let Some(path) = path_res
                .with_context(|| format!("error while iterating over glob {path:?}"))
                .log_if_err(found_err)
            {
                if path
                    .metadata()
                    .with_context(|| format!("failed to fetch metadata for {path:?}"))
                    .log_if_err(found_err)
                    .map_or(false, |m| m.is_file())
                {
                    f(&path, found_err).log_if_err(found_err);
                }
            }
        })
}
