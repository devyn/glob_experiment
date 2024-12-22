use anyhow::{anyhow, bail};

// mod matcher;
mod compiler;
mod parser;

fn main() -> anyhow::Result<()> {
    const USAGE: &str = "Usage: glob_experiment <pattern> <parse|compile>";

    let mut args = std::env::args_os().skip(1);

    let pattern_string = args.next().ok_or_else(|| anyhow!(USAGE))?;

    match args.next().map(|s| s.into_encoded_bytes()).as_deref() {
        Some(b"parse") => {
            let result = parser::parse(pattern_string);
            println!("{:#?}", result);
        }
        Some(b"compile") => {
            let pattern = parser::parse(pattern_string);
            let program = compiler::compile(&pattern)?;
            print!("{}", program);
        }
        _ => bail!(USAGE),
    }

    Ok(())
}
