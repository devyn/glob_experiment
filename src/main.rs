use std::{io::Write, path::PathBuf, sync::Arc};

use anyhow::{anyhow, bail};

mod compiler;
mod globber;
mod matcher;
mod parser;

fn main() -> anyhow::Result<()> {
    const USAGE: &str = "Usage: glob_experiment <pattern> <parse|compile|matches|glob> [path]";

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
        Some(b"glob") => {
            let pattern = parser::parse(pattern_string);
            let program = Arc::new(compiler::compile(&pattern)?);
            let mut stdout = std::io::stdout();
            for result in globber::glob(program) {
                let result = result?;
                stdout.write_all(result.as_os_str().as_encoded_bytes())?;
                stdout.write_all(b"\n")?;
            }
        }
        _ => bail!(USAGE),
    }

    Ok(())
}
