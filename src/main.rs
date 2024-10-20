mod matcher;
mod parser;

fn main() {
    let pattern_string = std::env::args_os()
        .nth(1)
        .expect("Usage: glob_experiment <pattern>");

    let result = parser::parse(pattern_string);

    println!("{:#?}", result);
}
