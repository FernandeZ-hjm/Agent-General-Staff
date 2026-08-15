use ags_cli::Cli;

fn main() {
    let result = std::env::current_dir()
        .map_err(|error| format!("cannot resolve current directory: {error}"))
        .and_then(|adapter_cwd| ags_cli::execute(Cli::parse().into_invocation(), adapter_cwd));
    match result {
        Ok((output, true)) => println!("{output}"),
        Ok((output, false)) => {
            println!("{output}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("ags: {error}");
            std::process::exit(1);
        }
    }
}
