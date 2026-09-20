//! Slash command registry — builtin name reservation, route dispatch, file
//! command expansion with `$1`/`$@`/`$ARGUMENTS`/`$@[start:length]`,
//! capability providers with `_shadowed` dedup, and floating autocomplete.
//!
//! Contract: `docs/research/agent-ux/README.md`, `omp://slash-command-internals.md`.

use std::collections::HashMap;

/// A builtin slash command.
#[derive(Debug, Clone)]
pub struct BuiltinCmd {
    pub name: &'static str,
    pub description: &'static str,
}

/// A file-backed slash command (template with substitution markers).
#[derive(Debug, Clone)]
pub struct FileCmd {
    pub name: String,
    pub template: String,
}

/// Capability provider (`omp://slash-command-internals.md`).
///
/// Default priorities: `native` 100, `omp-plugins` 90, `claude` 80,
/// `claude-plugins` / `agents` / `codex` 70, `opencode` 55.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashProvider {
    pub id: String,
    pub priority: u8,
}

/// One slash command in the merged catalog. Duplicate names from a
/// lower-priority provider set [`SlashCatalogItem::shadowed`] (`_shadowed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCatalogItem {
    pub name: String,
    pub description: String,
    pub provider: String,
    pub priority: u8,
    pub shadowed: bool,
}

#[derive(Debug, Clone)]
struct CapabilityCmd {
    provider: String,
    priority: u8,
    name: String,
    description: String,
    template: Option<String>,
}

/// Native builtin provider id and priority.
pub const SLASH_NATIVE: &str = "native";
/// Priority for native builtins and file templates.
pub const SLASH_NATIVE_PRIORITY: u8 = 100;

/// Routing result for a slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// The command name matches a registered builtin.  The string is the
    /// bare command name (without leading `/`).
    Builtin(String),
    /// The command was handled by a file/template expansion.  The string
    /// is the expanded prompt text.
    Expanded(String),
    /// The command was not recognised — passes through to the LLM as-is.
    Passthrough,
}

/// A completion suggestion for the floating autocomplete panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub name: String,
    pub description: String,
}

/// The slash command registry, matching the omp pipeline:
///
/// 1. Built-in names are reserved first (routes as `Builtin`).
/// 2. File commands with templates are expanded (`$1`, `$@`, `$ARGUMENTS`).
/// 3. Capability-provider templates (not `_shadowed`) expand next.
/// 4. Everything else → `Passthrough` (e.g. `/unknown` → LLM text).
#[derive(Debug, Clone)]
pub struct SlashRegistry {
    builtins: Vec<BuiltinCmd>,
    files: Vec<FileCmd>,
    /// Index: name → builtin for O(1) lookup.
    builtin_index: HashMap<&'static str, usize>,
    capabilities: Vec<CapabilityCmd>,
}

impl SlashRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        SlashRegistry {
            builtins: Vec::new(),
            files: Vec::new(),
            builtin_index: HashMap::new(),
            capabilities: Vec::new(),
        }
    }

    /// Register a builtin command.  Builtin names are reserved: a matching
    /// input will always route to `Builtin(name)` before any file expansion.
    ///
    /// # Panics
    ///
    /// Panics if a builtin with the same name already exists.
    pub fn register_builtin(&mut self, name: &'static str, description: &'static str) {
        assert!(
            !self.builtin_index.contains_key(name),
            "builtin '{name}' already registered"
        );
        let idx = self.builtins.len();
        self.builtins.push(BuiltinCmd { name, description });
        self.builtin_index.insert(name, idx);
    }

    /// Register a file-backed command with a template body.
    ///
    /// # Panics
    ///
    /// Panics if a file command with the same name already exists.
    pub fn register_file(&mut self, name: &str, template: &str) {
        assert!(
            !self.files.iter().any(|f| f.name == name),
            "file command '{name}' already registered"
        );
        self.files.push(FileCmd {
            name: name.to_owned(),
            template: template.to_owned(),
        });
    }

    /// Register a command from a named capability provider.
    ///
    /// Same name from two providers is allowed: the higher priority wins
    /// in [`SlashRegistry::catalog`] / [`SlashRegistry::complete`]; the
    /// rest are `_shadowed`.
    pub fn register_capability(
        &mut self,
        provider: &str,
        priority: u8,
        name: &str,
        description: &str,
        template: Option<&str>,
    ) {
        self.capabilities.push(CapabilityCmd {
            provider: provider.to_owned(),
            priority,
            name: name.to_owned(),
            description: description.to_owned(),
            template: template.map(str::to_owned),
        });
    }

    /// Route an input string.
    ///
    /// If the input starts with `/`, the command name (everything after `/`
    /// up to the first space or end) is matched:
    ///
    /// 1. Builtin name → `Builtin(name)`.
    /// 2. File command name → `Expanded(template)` with `$1`/`$@`/`$ARGUMENTS`
    ///    replaced by the command arguments.
    /// 3. Winning capability template → `Expanded`.
    /// 4. Otherwise → `Passthrough`.
    ///
    /// Input without a leading `/` is always `Passthrough`.
    pub fn route(&self, input: &str) -> Route {
        if !input.starts_with('/') {
            return Route::Passthrough;
        }
        let (cmd_name, args) = parse_slash(input);
        if self.builtin_index.contains_key(cmd_name) {
            return Route::Builtin(cmd_name.to_owned());
        }
        if let Some(file) = self.files.iter().find(|f| f.name == cmd_name) {
            let expanded = expand_template(&file.template, &args);
            return Route::Expanded(expanded);
        }
        if let Some(cap) = self.winner_capability(cmd_name)
            && let Some(template) = &cap.template
        {
            return Route::Expanded(expand_template(template, &args));
        }
        Route::Passthrough
    }

    /// Return completions for the given prefix.
    ///
    /// Matches catalog winners whose name starts with `prefix` (after
    /// stripping the leading `/`). `_shadowed` duplicates are omitted.
    pub fn complete(&self, prefix: &str) -> Vec<Completion> {
        let name = prefix.strip_prefix('/').unwrap_or(prefix);
        self.catalog()
            .into_iter()
            .filter(|item| !item.shadowed && item.name.starts_with(name))
            .map(|item| Completion {
                name: format!("/{}", item.name),
                description: item.description,
            })
            .collect()
    }

    /// Full catalog including `_shadowed` losers (`result.all` in OMP).
    pub fn catalog(&self) -> Vec<SlashCatalogItem> {
        struct Cand {
            name: String,
            description: String,
            provider: String,
            priority: u8,
            rank: u8,
            idx: usize,
        }
        let mut cands: Vec<Cand> = Vec::new();
        for (idx, cmd) in self.builtins.iter().enumerate() {
            cands.push(Cand {
                name: cmd.name.to_owned(),
                description: cmd.description.to_owned(),
                provider: SLASH_NATIVE.to_owned(),
                priority: SLASH_NATIVE_PRIORITY,
                rank: 0,
                idx,
            });
        }
        for (idx, file) in self.files.iter().enumerate() {
            cands.push(Cand {
                name: file.name.clone(),
                description: String::new(),
                provider: SLASH_NATIVE.to_owned(),
                priority: SLASH_NATIVE_PRIORITY,
                rank: 1,
                idx,
            });
        }
        for (idx, cap) in self.capabilities.iter().enumerate() {
            cands.push(Cand {
                name: cap.name.clone(),
                description: cap.description.clone(),
                provider: cap.provider.clone(),
                priority: cap.priority,
                rank: 2,
                idx,
            });
        }

        let mut winner: HashMap<String, (u8, u8, usize)> = HashMap::new();
        for c in &cands {
            match winner.get(&c.name) {
                Some(&(p, r, i))
                    if p > c.priority
                        || (p == c.priority && (r < c.rank || (r == c.rank && i <= c.idx))) => {}
                _ => {
                    winner.insert(c.name.clone(), (c.priority, c.rank, c.idx));
                }
            }
        }
        cands
            .into_iter()
            .map(|c| {
                let shadowed = winner.get(&c.name) != Some(&(c.priority, c.rank, c.idx));
                SlashCatalogItem {
                    name: c.name,
                    description: c.description,
                    provider: c.provider,
                    priority: c.priority,
                    shadowed,
                }
            })
            .collect()
    }

    fn winner_capability(&self, name: &str) -> Option<&CapabilityCmd> {
        let item = self
            .catalog()
            .into_iter()
            .find(|item| item.name == name && !item.shadowed)?;
        self.capabilities
            .iter()
            .find(|c| c.name == name && c.provider == item.provider && c.priority == item.priority)
    }

    /// Number of registered builtins.
    pub fn builtin_count(&self) -> usize {
        self.builtins.len()
    }

    /// Number of registered file commands.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Iterate over all builtin commands.
    pub fn builtins(&self) -> impl Iterator<Item = &BuiltinCmd> {
        self.builtins.iter()
    }
}

impl Default for SlashRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Slash parsing
// ---------------------------------------------------------------------------

/// Parse a slash command input into (command_name, args).
///
/// The command name is the token after `/` up to the first space or end.
/// Args are the remainder (if any) stripped of leading whitespace, then
/// split into quote-aware tokens.
///
/// Examples:
/// ```ignore
/// parse_slash("/help")         → ("help", [])
/// parse_slash("/model claude") → ("model", ["claude"])
/// parse_slash("/mode \"deep focus\"") → ("mode", ["deep focus"])
/// ```
pub fn parse_slash(input: &str) -> (&str, Vec<String>) {
    let input = input.strip_prefix('/').unwrap_or(input);
    let (name, rest) = match input.split_once(char::is_whitespace) {
        Some((n, r)) => (n, r.trim()),
        None => (input, ""),
    };
    let args = if rest.is_empty() {
        Vec::new()
    } else {
        parse_command_args(rest)
    };
    (name, args)
}

/// Quote-aware argument parser (no backslash escaping).
///
/// Splits into tokens separated by whitespace, respecting single and double
/// quotes.  Unmatched quotes are treated as literal.
pub fn parse_command_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for ch in input.chars() {
        match quote {
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
            }
            Some(q) if ch == q => {
                quote = None;
            }
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    args.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

// ---------------------------------------------------------------------------
// Template expansion
// ---------------------------------------------------------------------------

/// Expand a template by replacing `$N`, `$@`, `$@[start:length]`, `$ARGUMENTS`.
///
/// - `$1`, `$2`, … → positional argument (1-based; missing → empty)
/// - `$@` → all arguments joined by spaces
/// - `$@[start:length]` → a slice of arguments (`start` is 0-based)
/// - `$ARGUMENTS` → same as `$@`
pub fn expand_template(template: &str, args: &[String]) -> String {
    let mut result = String::new();
    let mut rest = template;

    while let Some(pos) = rest.find('$') {
        result.push_str(&rest[..pos]);
        rest = &rest[pos..];

        if rest.starts_with("$ARGUMENTS") {
            result.push_str(&args.join(" "));
            rest = &rest["$ARGUMENTS".len()..];
        } else if rest.starts_with("$@") {
            rest = &rest[2..];
            if let Some((start, length, consumed)) = parse_arg_slice(rest) {
                result.push_str(&join_args_slice(args, start, length));
                rest = &rest[consumed..];
            } else {
                result.push_str(&args.join(" "));
            }
        } else {
            let after = &rest[1..];
            let n_digits = after.bytes().take_while(u8::is_ascii_digit).count();
            if n_digits > 0 {
                if let Ok(idx) = after[..n_digits].parse::<usize>()
                    && idx >= 1
                    && let Some(arg) = args.get(idx - 1)
                {
                    result.push_str(arg);
                }
                rest = &rest[1 + n_digits..];
            } else {
                result.push('$');
                rest = &rest[1..];
            }
        }
    }

    result.push_str(rest);
    result
}

/// Parse `[start:length]` immediately after `$@`. `consumed` includes the brackets.
fn parse_arg_slice(after_at: &str) -> Option<(usize, usize, usize)> {
    let rest = after_at.strip_prefix('[')?;
    let close = rest.find(']')?;
    let inner = &rest[..close];
    let (a, b) = inner.split_once(':')?;
    if a.is_empty()
        || b.is_empty()
        || !a.bytes().all(|c| c.is_ascii_digit())
        || !b.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let start = a.parse().ok()?;
    let length = b.parse().ok()?;
    Some((start, length, close + 2))
}

fn join_args_slice(args: &[String], start: usize, length: usize) -> String {
    match args.get(start..) {
        Some(tail) => tail
            .iter()
            .take(length)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Route ------------------------------------------------------------

    #[test]
    fn builtin_route_returns_builtin() {
        let mut reg = SlashRegistry::new();
        reg.register_builtin("help", "Show help");
        assert_eq!(reg.route("/help"), Route::Builtin("help".into()));
    }

    #[test]
    fn unknown_slash_is_passthrough() {
        let reg = SlashRegistry::new();
        assert_eq!(reg.route("/unknown-xyz"), Route::Passthrough);
    }

    #[test]
    fn no_slash_is_passthrough() {
        let reg = SlashRegistry::new();
        assert_eq!(reg.route("hello world"), Route::Passthrough);
    }

    #[test]
    fn builtin_reserved_over_file() {
        let mut reg = SlashRegistry::new();
        reg.register_builtin("model", "Switch model");
        reg.register_file("model", "You are $ARGUMENTS");
        // Builtin wins over file command with same name.
        assert_eq!(reg.route("/model claude"), Route::Builtin("model".into()));
    }

    // ---- File expansion ---------------------------------------------------

    #[test]
    fn file_command_expands_arguments() {
        let mut reg = SlashRegistry::new();
        reg.register_file("translate", "Translate to English: $ARGUMENTS");
        assert_eq!(
            reg.route("/translate hello world"),
            Route::Expanded("Translate to English: hello world".into())
        );
    }

    #[test]
    fn file_command_expands_first_arg() {
        let mut reg = SlashRegistry::new();
        reg.register_file("say", "You said: $1");
        let result = reg.route("/say hello world");
        assert_eq!(result, Route::Expanded("You said: hello".into()));
    }

    // ---- Completion -------------------------------------------------------

    #[test]
    fn complete_prefix_matches_builtins() {
        let mut reg = SlashRegistry::new();
        reg.register_builtin("help", "Show help");
        reg.register_builtin("history", "View history");
        reg.register_builtin("model", "Switch model");

        let completions = reg.complete("/h");
        let names: Vec<&str> = completions.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"/help"));
        assert!(names.contains(&"/history"));
        assert!(!names.contains(&"/model"));
    }

    #[test]
    fn complete_no_slash_prefix() {
        let mut reg = SlashRegistry::new();
        reg.register_builtin("help", "Show help");
        let completions = reg.complete("help");
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].name, "/help");
    }

    #[test]
    fn complete_empty_prefix_returns_all() {
        let mut reg = SlashRegistry::new();
        reg.register_builtin("help", "Show help");
        reg.register_builtin("model", "Switch model");
        assert_eq!(reg.complete("/").len(), 2);
    }

    // ---- Parse slash ------------------------------------------------------

    #[test]
    fn parse_slash_bare() {
        let (name, args) = parse_slash("/help");
        assert_eq!(name, "help");
        assert!(args.is_empty());
    }

    #[test]
    fn parse_slash_with_args() {
        let (name, args) = parse_slash("/model claude-sonnet");
        assert_eq!(name, "model");
        assert_eq!(args, vec!["claude-sonnet"]);
    }

    #[test]
    fn parse_slash_quoted_args() {
        let (name, args) = parse_slash("/mode \"deep focus\"");
        assert_eq!(name, "mode");
        assert_eq!(args, vec!["deep focus"]);
    }

    // ---- Arg parser -------------------------------------------------------

    #[test]
    fn parse_command_args_empty() {
        assert!(parse_command_args("").is_empty());
    }

    #[test]
    fn parse_command_args_single_token() {
        assert_eq!(parse_command_args("hello"), vec!["hello"]);
    }

    #[test]
    fn parse_command_args_multiple() {
        assert_eq!(parse_command_args("a b c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_command_args_quoted() {
        assert_eq!(parse_command_args("a 'b c' d"), vec!["a", "b c", "d"]);
    }

    #[test]
    fn parse_command_args_double_quoted() {
        assert_eq!(parse_command_args("a \"b c\" d"), vec!["a", "b c", "d"]);
    }

    #[test]
    fn parse_command_args_unmatched_quote() {
        assert_eq!(parse_command_args("a 'b c"), vec!["a", "b c"]);
    }

    // ---- Template expansion -----------------------------------------------

    #[test]
    fn expand_no_markers() {
        let template = "Hello, world!";
        assert_eq!(expand_template(template, &[]), "Hello, world!");
    }

    #[test]
    fn expand_first_arg() {
        let template = "prefix $1 suffix";
        assert_eq!(
            expand_template(template, &["mid".into()]),
            "prefix mid suffix"
        );
    }

    #[test]
    fn expand_all_args() {
        let template = "You said: $@";
        assert_eq!(
            expand_template(template, &["hello".into(), "world".into()]),
            "You said: hello world"
        );
    }

    #[test]
    fn expand_arguments() {
        let template = "Translate: $ARGUMENTS";
        assert_eq!(
            expand_template(template, &["hello".into(), "world".into()]),
            "Translate: hello world"
        );
    }

    #[test]
    fn expand_nonexistent_var_preserves_dollar() {
        let template = "use $DOLLAR";
        assert_eq!(expand_template(template, &[]), "use $DOLLAR");
    }

    #[test]
    fn expand_mixed() {
        let template = "$1 said $@";
        assert_eq!(
            expand_template(template, &["alice".into(), "hello".into()]),
            "alice said alice hello"
        );
    }

    #[test]
    fn expand_second_arg() {
        let template = "$1 -> $2";
        assert_eq!(
            expand_template(template, &["a".into(), "b".into(), "c".into()]),
            "a -> b"
        );
    }

    #[test]
    fn expand_arg_slice() {
        let args = ["one".into(), "two".into(), "three".into(), "four".into()];
        assert_eq!(expand_template("X $@[1:2] Y", &args), "X two three Y");
        assert_eq!(expand_template("$@[0:1]", &args), "one");
        assert_eq!(expand_template("$@[3:8]", &args), "four");
        assert_eq!(expand_template("$@[9:1]", &args), "");
    }

    #[test]
    fn native_shadows_lower_priority_provider() {
        let mut reg = SlashRegistry::new();
        reg.register_builtin("help", "Show help");
        reg.register_capability("claude", 80, "help", "Claude help", Some("ignored $1"));
        let cat = reg.catalog();
        let native = cat
            .iter()
            .find(|i| i.provider == SLASH_NATIVE && i.name == "help")
            .expect("native");
        let claude = cat
            .iter()
            .find(|i| i.provider == "claude" && i.name == "help")
            .expect("claude");
        assert!(!native.shadowed);
        assert!(claude.shadowed);
        assert_eq!(reg.complete("/help").len(), 1);
        assert_eq!(reg.route("/help extra"), Route::Builtin("help".into()));
    }

    #[test]
    fn capability_template_routes_when_not_shadowed() {
        let mut reg = SlashRegistry::new();
        reg.register_capability("omp-plugins", 90, "greet", "Greet", Some("hello $1"));
        assert_eq!(
            reg.route("/greet world"),
            Route::Expanded("hello world".into())
        );
        let cat = reg.catalog();
        assert_eq!(cat.len(), 1);
        assert!(!cat[0].shadowed);
    }
}
