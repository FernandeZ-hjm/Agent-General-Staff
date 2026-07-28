use crate::cli::MemoryAction;
use std::path::Path;

pub(crate) fn run(action: MemoryAction) {
    match action {
        MemoryAction::Status { target } => {
            emit(&ags_evidence::memory::status(&memory_dir(&target)));
        }
        MemoryAction::Init { target } => {
            let status = ags_evidence::memory::init(&memory_dir(&target))
                .unwrap_or_else(|error| fail(error));
            emit(&status);
        }
        MemoryAction::Archive { receipt, target } => {
            let result = ags_evidence::memory::archive(&receipt, &memory_dir(&target))
                .unwrap_or_else(|error| fail(error));
            emit(&result);
        }
    }
}

fn memory_dir(target: &Path) -> std::path::PathBuf {
    ags_host_integration::project_memory_dir_at(target, &ags_platform::home_dir_or_temp())
}

fn emit<T: serde::Serialize>(value: &T) {
    println!("{}", ags_evidence::memory::render(value));
}

fn fail(error: String) -> ! {
    eprintln!("memory: {error}");
    std::process::exit(1)
}
