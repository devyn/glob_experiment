use std::path::PathBuf;

use anyhow::{anyhow, bail};

mod compiler;
mod matcher;
mod parser;

fn main() -> anyhow::Result<()> {
    const USAGE: &str = "Usage: glob_experiment <pattern> <parse|compile|matches> [path]";

    let mut args = std::env::args_os().skip(1);

    let pattern_string = args.next().ok_or_else(|| anyhow!(USAGE))?;

    match args.next().map(|s| s.into_encoded_bytes()).as_deref() {
        Some(b"parse") => {
            let pattern = parser::parse(pattern_string);
            println!("{:#?}", pattern);
        }
        Some(b"compile") => {
            let pattern = parser::parse(pattern_string);
            let program = compiler::compile(&pattern)?;
            print!("{}", program);
        }
        Some(b"matches") => {
            let path: PathBuf = args.next().ok_or_else(|| anyhow!(USAGE))?.into();
            let pattern = parser::parse(pattern_string);
            let program = compiler::compile(&pattern)?;
            let result = matcher::path_matches(&path, &program);
            print!("{:?}", result);
        }
        _ => bail!(USAGE),
    }

    Ok(())
}
