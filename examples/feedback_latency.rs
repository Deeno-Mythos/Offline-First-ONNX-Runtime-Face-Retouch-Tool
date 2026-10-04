use anyhow::{Context, Result};
use std::path::PathBuf;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let source = PathBuf::from(args.first().context("Supply a portrait image path")?);
    let output = PathBuf::from(args.get(1).context("Supply an output directory")?);
    hastur_retouch::ui::feedback_diagnostics::profile(
        &source,
        &output,
        args.iter().any(|a| a == "--baseline"),
    )
}
