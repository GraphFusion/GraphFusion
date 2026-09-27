//! Stateful GQL REPL. GQL framing uses the parser's lexer, including its
//! string escapes and comments; nested procedure bodies are never split on ';'.
use super::{print_outputs, CliResult};
use graphfusion::{
    catalog::ObjectKind,
    gql::{self, lexer::Lexer, token::TokenKind},
    Database, Session, TransactionStatus,
};
use rustyline::{error::ReadlineError, DefaultEditor};
use std::io::{self, BufRead, IsTerminal, Write};

const HELP: &str = r"Enter GQL terminated by ';'. Statements can span multiple lines.
  \help                Show this help
  \graphs              List committed graphs in the current schema
  \status              Show database, session and transaction status
  \read FILE           Execute a GQL file in this session (path may contain spaces)
  \explain on|off      Show execution plans after executing queries
  \checkpoint          Checkpoint and reclaim unused storage
  \clear               Discard unfinished input
  \quit, \q            Exit; any uncommitted transaction is rolled back
Ctrl-C clears input or cancels an executing query; Ctrl-D exits.
Up/Down and Ctrl-R recall this session's history. History is kept in memory.
";

pub async fn run(db: Database, path: Option<&str>, mut explain: bool) -> CliResult<()> {
    let terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut editor = if terminal {
        Some(DefaultEditor::new()?)
    } else {
        None
    };
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut session = db.session();
    let mut buffer = String::new();
    let mut failed = false;
    if terminal {
        println!(
            "GraphFusion {} — {}",
            env!("CARGO_PKG_VERSION"),
            path.unwrap_or("in-memory database")
        );
        println!("End GQL with ';'. Type \\help for help or \\quit to exit.");
    }
    loop {
        let line = if let Some(editor) = &mut editor {
            match editor.readline(prompt(&session, !buffer.is_empty())) {
                Ok(line) => {
                    editor.add_history_entry(line.as_str())?;
                    line
                }
                Err(ReadlineError::Interrupted) => {
                    buffer.clear();
                    println!("Input cleared.");
                    continue;
                }
                Err(ReadlineError::Eof) => break,
                Err(error) => return Err(error.into()),
            }
        } else {
            let mut line = String::new();
            if input.read_line(&mut line)? == 0 {
                break;
            }
            line
        };
        let trimmed = line.trim();
        if trimmed == "\\clear" {
            buffer.clear();
            continue;
        }
        if buffer.is_empty() && trimmed.starts_with('\\') {
            let (command, arg) = trimmed
                .split_once(char::is_whitespace)
                .unwrap_or((trimmed, ""));
            let arg = arg.trim();
            let result = match (command, arg) {
                ("\\q" | "\\quit", "") => break,
                ("\\help", "") => {
                    print!("{HELP}");
                    Ok(())
                }
                ("\\status", "") => {
                    println!(
                        "database={} schema={} graph={:?} transaction={:?}",
                        path.unwrap_or(":memory:"),
                        session.state().current_schema,
                        session.state().current_graph,
                        session.transaction_status()
                    );
                    Ok(())
                }
                ("\\graphs", "") => db
                    .with_catalog(|catalog| {
                        for graph in catalog
                            .children(session.state().current_schema)
                            .filter(|entry| entry.definition.kind() == ObjectKind::Graph)
                        {
                            println!("{}", graph.name);
                        }
                    })
                    .map_err(Into::into),
                ("\\checkpoint", "") => db
                    .checkpoint()
                    .map(|()| println!("OK checkpoint"))
                    .map_err(Into::into),
                ("\\explain", "on" | "off") => {
                    explain = arg == "on";
                    println!("Explain {arg} (queries and writes are executed).");
                    Ok(())
                }
                ("\\read", path) if !path.is_empty() => match std::fs::read_to_string(path) {
                    Ok(program) => execute(&mut session, &program, explain).await,
                    Err(error) => Err(error.into()),
                },
                _ => Err("unknown REPL command or arguments; use \\help".into()),
            };
            report(result, &mut failed);
        } else {
            buffer.push_str(&line);
            // readline strips the terminal's Enter; BufRead retains the input
            // newline. Preserve literal string contents in both input modes.
            if terminal {
                buffer.push('\n');
            }
            loop {
                match next_statement(&buffer) {
                    Ok(Some(end)) => {
                        let statement: String = buffer.drain(..end).collect();
                        if !is_empty(&statement) {
                            report(
                                execute(&mut session, &statement, explain).await,
                                &mut failed,
                            );
                        }
                        if session.state().closed {
                            break;
                        }
                    }
                    Ok(None) => {
                        if is_empty(&buffer) {
                            buffer.clear();
                        }
                        break;
                    }
                    Err(_) => {
                        // Route lexical failures through Session as well, so a
                        // bad input fails an explicit transaction consistently.
                        report(execute(&mut session, &buffer, explain).await, &mut failed);
                        buffer.clear();
                        break;
                    }
                }
            }
        }
        io::stdout().flush()?;
        if session.state().closed {
            break;
        }
    }
    if !is_empty(&buffer) {
        eprintln!("graphfusion: unfinished input discarded; terminate GQL with ';'");
        failed = true;
    }
    if session.transaction_status() != TransactionStatus::Idle {
        session.execute("ROLLBACK")?;
        eprintln!("graphfusion: uncommitted transaction rolled back");
        failed = true;
    }
    if failed && !terminal {
        return Err("REPL input contained errors or unfinished work".into());
    }
    Ok(())
}

fn prompt(session: &Session, pending: bool) -> &'static str {
    if pending {
        return "       ...> ";
    }
    match session.transaction_status() {
        TransactionStatus::Idle => "graphfusion> ",
        TransactionStatus::Active { .. } => "graphfusion[tx]> ",
        TransactionStatus::Failed { .. } => "graphfusion[failed]> ",
    }
}

async fn execute(session: &mut Session, input: &str, explain: bool) -> CliResult<()> {
    let outputs = tokio::select! {
        biased;
        signal = tokio::signal::ctrl_c() => {
            signal?;
            return Err("query interrupted; completed autocommits remain committed; check transaction status before continuing".into());
        }
        result = session.run(input) => result?,
    };
    print_outputs(outputs, explain)
}

fn report(result: CliResult<()>, failed: &mut bool) {
    if let Err(error) = result {
        eprintln!("graphfusion: {error}");
        *failed = true;
    }
}

/// Returns the byte boundary of the first top-level terminator. Lexical errors
/// that may be completed on a following line keep the buffer open.
fn next_statement(input: &str) -> gql::Result<Option<usize>> {
    let mut depth = 0usize;
    for token in Lexer::new(input) {
        let token = match token {
            Err(
                gql::Error::UnterminatedString { .. } | gql::Error::UnterminatedBlockComment { .. },
            ) => return Ok(None),
            other => other?,
        };
        match token.kind {
            TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => depth += 1,
            TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => {
                depth = depth.saturating_sub(1)
            }
            TokenKind::Semicolon if depth == 0 => return Ok(Some(token.offset + 1)),
            _ => (),
        }
    }
    Ok(None)
}

fn is_empty(input: &str) -> bool {
    Lexer::new(input).all(|token| matches!(token, Ok(t) if t.kind == TokenKind::Semicolon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_matches_gql_lexical_rules_and_nested_programs() {
        for statement in [
            "RETURN 'semi;--text' AS s;",
            "RETURN 'it''s;still quoted' AS s;",
            r"RETURN 'escaped\';value' AS s;",
            "RETURN `column;name` AS n;",
            "/* ; */ RETURN 42 AS n;",
            "// ;\nRETURN 42 AS n;",
            "-- ;\nRETURN 42 AS n;",
            "CALL { RETURN 1 AS n; RETURN 2 AS n; };",
        ] {
            let input = format!("{statement} RETURN 3 AS n;");
            assert_eq!(
                next_statement(&input).unwrap(),
                Some(statement.len()),
                "{input}"
            );
        }
        for incomplete in [
            "RETURN 'hello;",
            "/* unfinished;",
            "CALL { RETURN 1;",
            "RETURN 42 AS n",
        ] {
            assert_eq!(next_statement(incomplete).unwrap(), None);
        }
        assert!(is_empty("-- hello\n /* comment */ ;"));
        assert!(!is_empty("/* unfinished"));
        assert!(next_statement("RETURN @;").is_err());
    }
}
