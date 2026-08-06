//! Shared output / formatting helpers.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(format!(
                "unsupported output format '{other}'; expected text or json"
            )),
        }
    }
}

pub(crate) fn is_json(format: &str) -> bool {
    output_format_or_exit(format) == OutputFormat::Json
}

pub(crate) fn pretty_json<T: Serialize + ?Sized>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map_err(|error| format!("cannot serialize CLI JSON output: {error}"))
}

pub(crate) fn emit<T, F>(format: &str, value: &T, render_text: F)
where
    T: Serialize + ?Sized,
    F: FnOnce() -> String,
{
    let rendered = match output_format_or_exit(format) {
        OutputFormat::Json => pretty_json(value).unwrap_or_else(|error| output_error_exit(error)),
        OutputFormat::Text => render_text(),
    };
    println!("{rendered}");
}

pub(crate) fn emit_rendered<FJ, FT>(format: &str, render_json: FJ, render_text: FT)
where
    FJ: FnOnce() -> String,
    FT: FnOnce() -> String,
{
    let rendered = match output_format_or_exit(format) {
        OutputFormat::Json => render_json(),
        OutputFormat::Text => render_text(),
    };
    println!("{rendered}");
}

fn output_format_or_exit(format: &str) -> OutputFormat {
    OutputFormat::parse(format).unwrap_or_else(|error| output_error_exit(error))
}

fn output_error_exit(error: String) -> ! {
    eprintln!("ags: {error}");
    std::process::exit(2);
}

pub(crate) fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

pub(crate) fn error_exit(command: &str, error: impl AsRef<str>, format: &str, exit_code: i32) -> ! {
    let error = error.as_ref();
    if is_json(format) {
        println!(
            "{}",
            pretty_json(&serde_json::json!({
                "schema_version": "0.4.15-cli-error",
                "command": command,
                "error": {
                    "code": stable_error_code(error),
                    "message": error,
                }
            }))
            .unwrap_or_else(|serialization| format!(
                "{{\"error\":{{\"code\":\"serialization_failed\",\"message\":{serialization:?}}}}}"
            ))
        );
    } else {
        eprintln!("{command}: {error}");
    }
    std::process::exit(exit_code);
}

fn stable_error_code(error: &str) -> &'static str {
    for code in [
        "manifest_unknown_field",
        "manifest_yaml_syntax",
        "manifest_yaml_merge",
        "manifest_schema_invalid",
        "metadata_argument_requires_file",
        "metadata_file_not_found",
    ] {
        if error.contains(code) {
            return code;
        }
    }
    "command_failed"
}

// ── Receipt bridge (AGS-owned receipts) ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_is_closed_and_json_errors_are_not_silently_defaulted() {
        assert_eq!(OutputFormat::parse("text"), Ok(OutputFormat::Text));
        assert_eq!(OutputFormat::parse("json"), Ok(OutputFormat::Json));
        assert!(OutputFormat::parse("yaml").is_err());

        struct Fails;
        impl Serialize for Fails {
            fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(serde::ser::Error::custom("injected serialization failure"))
            }
        }
        assert_eq!(
            pretty_json(&Fails).unwrap_err(),
            "cannot serialize CLI JSON output: injected serialization failure"
        );
        assert_eq!(
            stable_error_code("cannot parse file: manifest_unknown_field: field=notes"),
            "manifest_unknown_field"
        );
        assert_eq!(
            stable_error_code("metadata_argument_requires_file: inline YAML"),
            "metadata_argument_requires_file"
        );
    }
}
