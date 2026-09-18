//! Bounded command-buffer and console-variable ownership for the Rust host.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

pub const CVAR_READ_ONLY: u32 = 1 << 0;
pub const MAX_NAME_BYTES: usize = 128;
pub const MAX_VALUE_BYTES: usize = 4096;
pub const MAX_COMMAND_BYTES: usize = 4096;
pub const MAX_COMMAND_ARGS: usize = 64;
pub const MAX_QUEUED_COMMANDS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    EmptyName,
    InvalidName,
    NameTooLong,
    ValueTooLong,
    AlreadyRegistered,
    NotFound,
    ReadOnly,
    UnterminatedQuote,
    InvalidEscape,
    CommandTooLong,
    TooManyArguments,
    QueueFull,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyName => "console name is empty",
            Self::InvalidName => "console name contains invalid characters",
            Self::NameTooLong => "console name exceeds the byte limit",
            Self::ValueTooLong => "console value exceeds the byte limit",
            Self::AlreadyRegistered => "console variable is already registered differently",
            Self::NotFound => "console variable was not found",
            Self::ReadOnly => "console variable is read-only",
            Self::UnterminatedQuote => "command contains an unterminated quote",
            Self::InvalidEscape => "command contains an invalid quoted escape",
            Self::CommandTooLong => "command exceeds the byte limit",
            Self::TooManyArguments => "command has too many arguments",
            Self::QueueFull => "command queue is full",
        };
        f.write_str(message)
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    name: String,
    default_value: String,
    value: String,
    flags: u32,
    generation: u64,
}

impl Variable {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn default_value(&self) -> &str {
        &self.default_value
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn flags(&self) -> u32 {
        self.flags
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    args: Vec<String>,
}

impl Command {
    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn canonical(&self) -> String {
        let mut output = String::new();
        for (index, argument) in self.args.iter().enumerate() {
            if index != 0 {
                output.push(' ');
            }
            if argument.is_empty()
                || argument
                    .bytes()
                    .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b';' | b'"' | b'\\'))
            {
                output.push('"');
                for character in argument.chars() {
                    if matches!(character, '"' | '\\') {
                        output.push('\\');
                    }
                    output.push(character);
                }
                output.push('"');
            } else {
                output.push_str(argument);
            }
        }
        output
    }
}

#[derive(Debug, Default, Clone)]
pub struct Console {
    variables: BTreeMap<String, Variable>,
    commands: VecDeque<Command>,
}

impl Console {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: &str, default_value: &str, flags: u32) -> Result<bool, Error> {
        let key = canonical_name(name)?;
        validate_value(default_value)?;
        if let Some(existing) = self.variables.get(&key) {
            if existing.default_value == default_value && existing.flags == flags {
                return Ok(false);
            }
            return Err(Error::AlreadyRegistered);
        }
        self.variables.insert(
            key.clone(),
            Variable {
                name: key,
                default_value: default_value.to_owned(),
                value: default_value.to_owned(),
                flags,
                generation: 0,
            },
        );
        Ok(true)
    }

    pub fn variable(&self, name: &str) -> Result<&Variable, Error> {
        let key = canonical_name(name)?;
        self.variables.get(&key).ok_or(Error::NotFound)
    }

    pub fn set(&mut self, name: &str, value: &str) -> Result<u64, Error> {
        validate_value(value)?;
        let key = canonical_name(name)?;
        let variable = self.variables.get_mut(&key).ok_or(Error::NotFound)?;
        if variable.flags & CVAR_READ_ONLY != 0 {
            return Err(Error::ReadOnly);
        }
        if variable.value != value {
            variable.value.clear();
            variable.value.push_str(value);
            variable.generation = variable.generation.saturating_add(1);
        }
        Ok(variable.generation)
    }

    pub fn reset(&mut self, name: &str) -> Result<u64, Error> {
        let key = canonical_name(name)?;
        let variable = self.variables.get_mut(&key).ok_or(Error::NotFound)?;
        if variable.flags & CVAR_READ_ONLY != 0 {
            return Err(Error::ReadOnly);
        }
        if variable.value != variable.default_value {
            variable.value.clone_from(&variable.default_value);
            variable.generation = variable.generation.saturating_add(1);
        }
        Ok(variable.generation)
    }

    pub fn enqueue(&mut self, script: &str) -> Result<usize, Error> {
        let parsed = parse_commands(script)?;
        if self.commands.len().saturating_add(parsed.len()) > MAX_QUEUED_COMMANDS {
            return Err(Error::QueueFull);
        }
        let count = parsed.len();
        self.commands.extend(parsed);
        Ok(count)
    }

    pub fn pop(&mut self) -> Option<Command> {
        self.commands.pop_front()
    }

    pub fn front(&self) -> Option<&Command> {
        self.commands.front()
    }

    pub fn queued_command_count(&self) -> usize {
        self.commands.len()
    }
}

fn canonical_name(name: &str) -> Result<String, Error> {
    if name.is_empty() {
        return Err(Error::EmptyName);
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(Error::NameTooLong);
    }
    let mut bytes = name.bytes();
    let first = bytes.next().expect("nonempty name");
    if !(first.is_ascii_alphabetic() || first == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        return Err(Error::InvalidName);
    }
    Ok(name.to_ascii_lowercase())
}

fn validate_value(value: &str) -> Result<(), Error> {
    if value.len() > MAX_VALUE_BYTES {
        Err(Error::ValueTooLong)
    } else {
        Ok(())
    }
}

fn parse_commands(script: &str) -> Result<Vec<Command>, Error> {
    let mut commands = Vec::new();
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut in_comment = false;
    let mut escaped = false;
    let mut token_started = false;

    let finish_argument = |args: &mut Vec<String>,
                           current: &mut String,
                           token_started: &mut bool|
     -> Result<(), Error> {
        if *token_started {
            if args.len() >= MAX_COMMAND_ARGS {
                return Err(Error::TooManyArguments);
            }
            args.push(std::mem::take(current));
            *token_started = false;
        }
        Ok(())
    };
    let finish_command = |commands: &mut Vec<Command>,
                          args: &mut Vec<String>,
                          current: &mut String,
                          token_started: &mut bool|
     -> Result<(), Error> {
        finish_argument(args, current, token_started)?;
        if !args.is_empty() {
            commands.push(Command {
                args: std::mem::take(args),
            });
        }
        Ok(())
    };

    let characters: Vec<char> = script.chars().collect();
    let mut index = 0usize;
    while index < characters.len() {
        let character = characters[index];
        if in_comment {
            if character == '\n' {
                in_comment = false;
                finish_command(&mut commands, &mut args, &mut current, &mut token_started)?;
            }
            index += 1;
            continue;
        }
        if escaped {
            if !matches!(character, '"' | '\\') {
                return Err(Error::InvalidEscape);
            }
            current.push(character);
            escaped = false;
            index += 1;
            continue;
        }
        if in_quotes {
            match character {
                '\\' => escaped = true,
                '"' => in_quotes = false,
                _ => current.push(character),
            }
            index += 1;
            continue;
        }
        if character == '/' && characters.get(index + 1) == Some(&'/') {
            in_comment = true;
            index += 2;
            continue;
        }
        match character {
            '"' => {
                in_quotes = true;
                token_started = true;
            }
            ';' | '\n' | '\r' => {
                finish_command(&mut commands, &mut args, &mut current, &mut token_started)?
            }
            character if character.is_whitespace() => {
                finish_argument(&mut args, &mut current, &mut token_started)?
            }
            _ => {
                current.push(character);
                token_started = true;
            }
        }
        if current.len() > MAX_COMMAND_BYTES {
            return Err(Error::CommandTooLong);
        }
        index += 1;
    }
    if escaped {
        return Err(Error::InvalidEscape);
    }
    if in_quotes {
        return Err(Error::UnterminatedQuote);
    }
    finish_command(&mut commands, &mut args, &mut current, &mut token_started)?;
    for command in &commands {
        if command.canonical().len() > MAX_COMMAND_BYTES {
            return Err(Error::CommandTooLong);
        }
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_case_insensitive_variables_and_generations() {
        let mut console = Console::new();
        assert_eq!(console.register("Host_Timescale", "1", 0), Ok(true));
        assert_eq!(console.register("host_timescale", "1", 0), Ok(false));
        assert_eq!(console.variable("HOST_TIMESCALE").unwrap().value(), "1");
        assert_eq!(console.set("host_timescale", "0.5"), Ok(1));
        assert_eq!(console.set("HOST_TIMESCALE", "0.5"), Ok(1));
        assert_eq!(console.reset("host_timescale"), Ok(2));
        assert_eq!(console.variable("host_timescale").unwrap().value(), "1");
    }

    #[test]
    fn enforces_names_values_and_read_only_variables() {
        let mut console = Console::new();
        assert_eq!(console.register("1bad", "", 0), Err(Error::InvalidName));
        assert_eq!(
            console.register("read_only", "yes", CVAR_READ_ONLY),
            Ok(true)
        );
        assert_eq!(console.set("read_only", "no"), Err(Error::ReadOnly));
        assert_eq!(console.variable("missing"), Err(Error::NotFound));
        assert_eq!(
            console.register("read_only", "different", CVAR_READ_ONLY),
            Err(Error::AlreadyRegistered)
        );
    }

    #[test]
    fn tokenizes_and_canonicalizes_bounded_source_commands() {
        let mut console = Console::new();
        assert_eq!(
            console.enqueue("echo \"hello world\"; map d1_trainstation_01 // ignored\nquit"),
            Ok(3)
        );
        assert_eq!(
            console.pop().unwrap().args(),
            &["echo".to_owned(), "hello world".to_owned()]
        );
        assert_eq!(console.pop().unwrap().canonical(), "map d1_trainstation_01");
        assert_eq!(console.pop().unwrap().canonical(), "quit");
        assert_eq!(console.pop(), None);

        assert_eq!(console.enqueue("echo \"\" \"a\\\"b\""), Ok(1));
        assert_eq!(
            console.pop().unwrap().args(),
            &["echo".to_owned(), "".to_owned(), "a\"b".to_owned()]
        );
    }

    #[test]
    fn rejects_malformed_and_excessive_commands_atomically() {
        let mut console = Console::new();
        assert_eq!(
            console.enqueue("echo \"unterminated"),
            Err(Error::UnterminatedQuote)
        );
        assert_eq!(console.queued_command_count(), 0);
        assert_eq!(
            console.enqueue(&format!("echo {}", "x".repeat(MAX_COMMAND_BYTES + 1))),
            Err(Error::CommandTooLong)
        );
        assert_eq!(console.queued_command_count(), 0);
    }
}
