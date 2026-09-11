//! Help text, captured from the JS CLI itself.
//!
//! The JS CLI is commander-based and renders usage for free; this port has a
//! hand-rolled dispatch, so until now `bukio --help` printed only the global
//! flags and `bukio bank --help` answered UNKNOWN_COMMAND. scripts/help-tree.mjs
//! walks commander's own output and writes src/help.json, which is embedded
//! here (include_str!, like profiles.json) and checked by a drift test — a
//! hand-maintained second copy of the command tree would drift the first time a
//! command is renamed.
use std::collections::HashMap;
use std::sync::OnceLock;

static HELP_JSON: &str = include_str!("help.json");

fn table() -> &'static HashMap<String, String> {
    static T: OnceLock<HashMap<String, String>> = OnceLock::new();
    T.get_or_init(|| serde_json::from_str(HELP_JSON).expect("help.json is valid JSON"))
}

/// The captured help text for a command path ("" is the root), if it exists.
pub fn text_for(path: &str) -> Option<&'static str> {
    table().get(path).map(String::as_str)
}

/// True when `path` names a command group that has children.
pub fn has_children(path: &str) -> bool {
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path} ")
    };
    table().keys().any(|k| k.starts_with(&prefix) && *k != path)
}

/// Global flags that take a value: reconstructing the command path from argv
/// needs to skip their argument (commander knows its option arity; this is the
/// port's whole global surface, and a test pins it against the root help).
const VALUE_FLAGS: [&str; 5] = ["--db", "--actor", "--locale", "--sign-key", "--server"];

/// The help text for the command named in argv — the LONGEST known prefix, so
/// `bukio --json entry add -h` and `bukio entry add --name Foo -h` both land on
/// `entry add`. Falls back to the root help, which always exists.
pub fn resolve(argv: &[String]) -> &'static str {
    let mut words: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if a.starts_with('-') {
            if VALUE_FLAGS.contains(&a.as_str()) {
                i += 1; // its value is not a command word
            }
        } else {
            words.push(a.clone());
        }
        i += 1;
    }
    // longest prefix that names a command
    for n in (0..=words.len()).rev() {
        let candidate = words[..n].join(" ");
        if table().contains_key(&candidate) {
            return table().get(&candidate).map(String::as_str).unwrap_or("");
        }
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn captured_help_covers_the_command_tree() {
        assert!(table().len() > 100, "the walk found the tree");
        assert!(text_for("").unwrap().contains("Usage: bukio"));
        assert!(text_for("bank").unwrap().contains("Usage: bukio bank"));
        assert!(text_for("payments payables add")
            .unwrap()
            .contains("Usage: bukio payments payables add"));
        assert!(has_children("bank"));
        assert!(!has_children("bank add"));
    }

    #[test]
    fn resolution_picks_the_longest_known_path() {
        assert!(resolve(&v(&["--help"])).contains("Usage: bukio [options]"));
        assert!(resolve(&v(&["bank", "--help"])).contains("Usage: bukio bank"));
        assert!(resolve(&v(&["bank", "add", "-h"])).contains("Usage: bukio bank add"));
        // global flags anywhere, and their values are not command words
        assert!(resolve(&v(&["--json", "entry", "add", "-h"])).contains("Usage: bukio entry add"));
        assert!(
            resolve(&v(&["--db", "/tmp/x.db", "invoice", "pay", "--help"]))
                .contains("Usage: bukio invoice pay")
        );
        // an option value the walk cannot know about does not win over the path
        assert!(resolve(&v(&["entry", "add", "--name", "Foo", "-h"]))
            .contains("Usage: bukio entry add"));
    }
}
