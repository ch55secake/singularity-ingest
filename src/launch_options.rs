//! Adds `-condebug -conclearlog` to Deadlock's Steam launch options.
//!
//! With `-condebug` the game writes its console to `game/citadel/console.log`,
//! which is how the collector sees a match start; `-conclearlog` starts that
//! file afresh on each launch so it doesn't grow forever. Steam keeps launch options in
//! `userdata/<account id>/config/localconfig.vdf` and rewrites that file when it
//! exits, so the edit only sticks while Steam is closed.

use core::ops::Range;
use std::path::{Path, PathBuf};

use crate::game::DEADLOCK_APP_ID;

pub(crate) const CONSOLE_LOG_FLAG: &str = "-condebug";
const LAUNCH_FLAGS: [&str; 2] = [CONSOLE_LOG_FLAG, "-conclearlog"];
const APPS_PATH: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];

/// A key and its value in a VDF file, with the byte ranges needed to edit it in place.
#[derive(Debug)]
struct Node {
    key: String,
    value: Value,
}

#[derive(Debug)]
enum Value {
    /// The raw (still escaped) text between the quotes.
    Text(Range<usize>),
    /// `open` is the position just after `{`.
    Block { open: usize, children: Vec<Node> },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Change {
    AlreadySet,
    Updated(String),
}

/// The local config file of the Steam user who logged in most recently.
pub(crate) fn local_config_path(steam_path: &Path, account_id: u32) -> PathBuf {
    steam_path
        .join("userdata")
        .join(account_id.to_string())
        .join("config")
        .join("localconfig.vdf")
}

/// Whether Deadlock's launch options in this file already include `-condebug`.
pub(crate) fn has_console_log_flag(contents: &str) -> Option<bool> {
    let nodes = parse(contents)?;
    let app = find_block(&nodes, &APPS_PATH)
        .and_then(|apps| find(apps, &DEADLOCK_APP_ID.to_string()))
        .and_then(|app| children(app));
    let options = app
        .and_then(|app| find(app, "LaunchOptions"))
        .and_then(|node| match &node.value {
            Value::Text(range) => Some(&contents[range.clone()]),
            Value::Block { .. } => None,
        });
    Some(options.is_some_and(|options| contains_flag(options, CONSOLE_LOG_FLAG)))
}

/// Returns the file with `-condebug -conclearlog` added to Deadlock's launch options.
pub(crate) fn add_console_log_flag(contents: &str) -> Result<Change, String> {
    let nodes = parse(contents).ok_or("localconfig.vdf could not be parsed")?;
    let apps_node = find_path(&nodes, &APPS_PATH).ok_or("localconfig.vdf has no apps section")?;
    let Value::Block {
        open: apps_open,
        children: apps,
    } = &apps_node.value
    else {
        return Err("the apps section in localconfig.vdf is not a block".into());
    };
    let depth = APPS_PATH.len();
    let app_id = DEADLOCK_APP_ID.to_string();

    let (position, replace_to, insert) = match find(apps, &app_id).map(|app| &app.value) {
        Some(Value::Block { open, children }) => match find(children, "LaunchOptions") {
            Some(Node {
                value: Value::Text(range),
                ..
            }) => {
                let current = &contents[range.clone()];
                let missing = LAUNCH_FLAGS
                    .into_iter()
                    .filter(|flag| !contains_flag(current, flag))
                    .collect::<Vec<_>>();
                if missing.is_empty() {
                    return Ok(Change::AlreadySet);
                }
                let options = [current.trim_end()]
                    .into_iter()
                    .chain(missing)
                    .filter(|part| !part.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                (range.start, range.end, options)
            }
            Some(_) => return Err("LaunchOptions in localconfig.vdf is not text".into()),
            None => (
                *open,
                *open,
                format!(
                    "\n{}\"LaunchOptions\"\t\t\"{}\"",
                    tabs(depth + 1),
                    LAUNCH_FLAGS.join(" ")
                ),
            ),
        },
        Some(Value::Text(_)) => {
            return Err("Deadlock's entry in localconfig.vdf is not a block".into());
        }
        None => (
            *apps_open,
            *apps_open,
            format!(
                "\n{indent}\"{app_id}\"\n{indent}{{\n{inner}\"LaunchOptions\"\t\t\"{flags}\"\n{indent}}}",
                indent = tabs(depth),
                inner = tabs(depth + 1),
                flags = LAUNCH_FLAGS.join(" "),
            ),
        ),
    };
    let mut updated = String::with_capacity(contents.len() + insert.len());
    updated.push_str(&contents[..position]);
    updated.push_str(&insert);
    updated.push_str(&contents[replace_to..]);
    Ok(Change::Updated(updated))
}

fn contains_flag(options: &str, flag: &str) -> bool {
    options
        .split_whitespace()
        .any(|word| word.eq_ignore_ascii_case(flag))
}

fn tabs(count: usize) -> String {
    "\t".repeat(count)
}

fn find<'a>(nodes: &'a [Node], key: &str) -> Option<&'a Node> {
    nodes.iter().find(|node| node.key.eq_ignore_ascii_case(key))
}

fn children(node: &Node) -> Option<&[Node]> {
    match &node.value {
        Value::Block { children, .. } => Some(children),
        Value::Text(_) => None,
    }
}

fn find_path<'a>(nodes: &'a [Node], path: &[&str]) -> Option<&'a Node> {
    let (first, rest) = path.split_first()?;
    let node = find(nodes, first)?;
    if rest.is_empty() {
        Some(node)
    } else {
        find_path(children(node)?, rest)
    }
}

fn find_block<'a>(nodes: &'a [Node], path: &[&str]) -> Option<&'a [Node]> {
    children(find_path(nodes, path)?)
}

enum Token {
    Text(Range<usize>),
    Open(usize),
    Close,
}

fn tokenize(contents: &str) -> Option<Vec<Token>> {
    let bytes = contents.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let start = index + 1;
                index = start;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                if index >= bytes.len() {
                    return None;
                }
                tokens.push(Token::Text(start..index));
                index += 1;
            }
            b'{' => {
                tokens.push(Token::Open(index + 1));
                index += 1;
            }
            b'}' => {
                tokens.push(Token::Close);
                index += 1;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            _ => {
                // Unquoted token: read until whitespace or a brace.
                let start = index;
                while index < bytes.len()
                    && !bytes[index].is_ascii_whitespace()
                    && !matches!(bytes[index], b'{' | b'}' | b'"')
                {
                    index += 1;
                }
                tokens.push(Token::Text(start..index));
            }
        }
    }
    Some(tokens)
}

fn parse(contents: &str) -> Option<Vec<Node>> {
    let tokens = tokenize(contents)?;
    let mut position = 0;
    let nodes = parse_nodes(contents, &tokens, &mut position)?;
    (position == tokens.len()).then_some(nodes)
}

fn parse_nodes(contents: &str, tokens: &[Token], position: &mut usize) -> Option<Vec<Node>> {
    let mut nodes = Vec::new();
    while let Some(token) = tokens.get(*position) {
        let Token::Text(key) = token else {
            break;
        };
        *position += 1;
        let value = match tokens.get(*position)? {
            Token::Text(range) => {
                *position += 1;
                Value::Text(range.clone())
            }
            Token::Open(open) => {
                *position += 1;
                let children = parse_nodes(contents, tokens, position)?;
                if !matches!(tokens.get(*position)?, Token::Close) {
                    return None;
                }
                *position += 1;
                Value::Block {
                    open: *open,
                    children,
                }
            }
            Token::Close => return None,
        };
        nodes.push(Node {
            key: contents[key.clone()].to_string(),
            value,
        });
    }
    Some(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WITHOUT_OPTIONS: &str = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"1422450\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1792079546\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n\t\"apps\"\n\t{\n\t\t\"1422450\"\t\t\"00696e\"\n\t}\n}\n";

    fn updated(contents: &str) -> String {
        match add_console_log_flag(contents).unwrap() {
            Change::Updated(updated) => updated,
            Change::AlreadySet => panic!("expected a change"),
        }
    }

    #[test]
    fn adds_launch_options_to_an_existing_app_block() {
        let result = updated(WITHOUT_OPTIONS);
        assert!(result.contains(
            "\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-condebug -conclearlog\"\n\t\t\t\t\t\t\"LastPlayed\""
        ));
        // The unrelated top-level "apps" entry is left alone.
        assert!(result.contains("\t\t\"1422450\"\t\t\"00696e\""));
        assert_eq!(has_console_log_flag(&result), Some(true));
        assert_eq!(add_console_log_flag(&result).unwrap(), Change::AlreadySet);
    }

    #[test]
    fn appends_to_existing_launch_options() {
        let contents = WITHOUT_OPTIONS.replace(
            "\"LastPlayed\"",
            "\"LaunchOptions\"\t\t\"-novid +fps_max 240\"\n\t\t\t\t\t\t\"LastPlayed\"",
        );
        assert_eq!(has_console_log_flag(&contents), Some(false));
        let result = updated(&contents);
        assert!(
            result.contains("\"LaunchOptions\"\t\t\"-novid +fps_max 240 -condebug -conclearlog\"")
        );
    }

    #[test]
    fn keeps_launch_options_that_already_have_the_flag() {
        let contents = WITHOUT_OPTIONS.replace(
            "\"LastPlayed\"",
            "\"LaunchOptions\"\t\t\"-conclearlog -condebug -novid\"\n\t\t\t\t\t\t\"LastPlayed\"",
        );
        assert_eq!(add_console_log_flag(&contents).unwrap(), Change::AlreadySet);
    }

    #[test]
    fn adds_only_the_missing_flag() {
        let contents = WITHOUT_OPTIONS.replace(
            "\"LastPlayed\"",
            "\"LaunchOptions\"\t\t\"-condebug\"\n\t\t\t\t\t\t\"LastPlayed\"",
        );
        assert_eq!(has_console_log_flag(&contents), Some(true));
        let result = updated(&contents);
        assert!(result.contains("\"LaunchOptions\"\t\t\"-condebug -conclearlog\""));
    }

    #[test]
    fn creates_the_app_block_when_deadlock_has_none() {
        let contents = WITHOUT_OPTIONS.replace("\"1422450\"\n", "\"730\"\n");
        let result = updated(&contents);
        assert!(result.contains("\t\t\t\t{\n\t\t\t\t\t\"1422450\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-condebug -conclearlog\"\n\t\t\t\t\t}"));
        assert_eq!(has_console_log_flag(&result), Some(true));
    }

    #[test]
    fn handles_escaped_quotes_in_values() {
        let contents = WITHOUT_OPTIONS.replace(
            "\"LastPlayed\"",
            "\"LaunchOptions\"\t\t\"+exec \\\"my cfg\\\"\"\n\t\t\t\t\t\t\"LastPlayed\"",
        );
        let result = updated(&contents);
        assert!(
            result.contains("\"LaunchOptions\"\t\t\"+exec \\\"my cfg\\\" -condebug -conclearlog\"")
        );
    }

    #[test]
    fn rejects_files_it_cannot_parse() {
        assert!(add_console_log_flag("\"UserLocalConfigStore\"\n{\n").is_err());
        assert!(add_console_log_flag("\"Other\"\n{\n}\n").is_err());
    }
}
