use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("ags-mcp: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [] => ags_mcp::run_stdio_adapter(),
        [mode] if mode == "stdio" => ags_mcp::run_stdio_adapter(),
        [mode, workspace_flag, workspace]
            if mode == "daemon" && workspace_flag == "--workspace" =>
        {
            ags_mcp::run_workspace_daemon(&PathBuf::from(workspace))
        }
        _ => Err("usage: ags-mcp [stdio | daemon --workspace <path>]".to_string()),
    }
}
