//! Parse GQL files without executing database operations.
use std::ffi::OsString;
use std::io::{Read, Write};

use gql_parser::{format_ast, parse};

const USAGE: &str =
    "usage: parse_file [--dump-ast] [--] FILE [FILE ...]\nUse - to read GQL from stdin.";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(
        std::env::args_os().skip(1),
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
    )
}

fn run(
    args: impl IntoIterator<Item = OsString>,
    stdin: &mut dyn Read,
    output: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut dump_ast = false;
    let mut positional = false;
    let mut files = Vec::new();
    for arg in args {
        if !positional && arg == "--" {
            positional = true;
        } else if !positional && arg == "--dump-ast" {
            dump_ast = true;
        } else if !positional && (arg == "--help" || arg == "-h") {
            writeln!(output, "{USAGE}")?;
            return Ok(());
        } else if !positional && arg != "-" && arg.to_string_lossy().starts_with('-') {
            return Err(format!("unknown option {arg:?}\n{USAGE}").into());
        } else {
            files.push(arg);
        }
    }
    if files.is_empty() {
        return Err(USAGE.into());
    }
    if files.iter().filter(|file| *file == "-").count() > 1 {
        return Err("stdin (-) may only be read once".into());
    }
    let multiple = files.len() > 1;
    for file in files {
        let input = if file == "-" {
            let mut input = String::new();
            stdin.read_to_string(&mut input)?;
            input
        } else {
            std::fs::read_to_string(&file).map_err(|error| format!("{file:?}: {error}"))?
        };
        let program = parse(&input).map_err(|error| format!("{file:?}: {error}"))?;
        if dump_ast {
            if multiple {
                writeln!(output, "==> {file:?} <==")?;
            }
            write!(output, "{}", format_ast(&program))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoke(args: &[&str], input: &str) -> Result<String, Box<dyn std::error::Error>> {
        let mut output = Vec::new();
        run(
            args.iter().map(OsString::from),
            &mut input.as_bytes(),
            &mut output,
        )?;
        Ok(String::from_utf8(output)?)
    }

    #[test]
    fn validation_is_quiet() {
        assert_eq!(invoke(&["-"], "RETURN 1 AS value").unwrap(), "");
    }

    #[test]
    fn dumps_stdin_using_public_formatter() {
        let input = "RETURN 1 + 2 * 3 AS value";
        assert_eq!(
            invoke(&["--dump-ast", "-"], input).unwrap(),
            format_ast(&parse(input).unwrap())
        );
    }

    #[test]
    fn errors_include_the_input_source() {
        let err = invoke(&["--dump-ast", "-"], "RETURN '")
            .unwrap_err()
            .to_string();
        assert!(err.contains("\"-\": unterminated string"), "{err}");
    }

    #[test]
    fn help_does_not_require_input() {
        assert_eq!(invoke(&["--help"], "").unwrap(), format!("{USAGE}\n"));
    }

    #[test]
    fn rejects_missing_input_unknown_options_and_repeated_stdin() {
        for args in [
            &[][..],
            &["--dump-ast"][..],
            &["--unknown"][..],
            &["-", "-"][..],
        ] {
            assert!(invoke(args, "RETURN 1 AS value").is_err(), "{args:?}");
        }
    }

    #[test]
    fn propagates_output_errors() {
        struct BrokenWriter;
        impl Write for BrokenWriter {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let err = run(
            ["--dump-ast".into(), "-".into()],
            &mut b"RETURN 1 AS value".as_slice(),
            &mut BrokenWriter,
        )
        .unwrap_err();
        assert_eq!(
            err.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::BrokenPipe
        );
    }
}
