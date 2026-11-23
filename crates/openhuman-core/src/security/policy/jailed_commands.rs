//! Shell commands that will run inside the kernel sandbox.
//!
//! TinySecurity's protected-path floor is lexical: it refuses a command when
//! any word of its text names a parent folder (`..`), a critical root (`/etc`,
//! `/proc`, ...) or a credential store, whatever the command does with it.
//! `cd ..`, `sed 's/../X/'` and `cat /etc/os-release` are all refused. For a
//! command that runs on the host that floor is the only thing between the
//! model and those paths. A command that runs inside the jail (Landlock /
//! bubblewrap through TinyBox, or a container) can reach only what its plan
//! grants, and the plan keeps the floor, the credential deny list and the
//! core's own state out of every grant (`sandbox::spawn_plan`). There the
//! words in the command text add nothing, so the floor is waived.
//!
//! Only that one refusal is waived. The module answers the floor before the
//! rest of its admission (the tier's class gate, the hidden-execution
//! defence, the hourly budget), so a floor refusal says nothing about those.
//! [`SecurityPolicy::check_jailed_command`] asks again with the path words
//! replaced by a neutral word and the original class declared, so every other
//! refusal still applies and the class can only go up. A command whose
//! protected words cannot be neutralized stays refused.
//!
//! The waiver holds only while the command really runs in the jail: a caller
//! that ends up on the host after all must refuse it ([`JailedVerdict`]).

use super::native_commands::{CommandFlow, CommandQuery, CommandVerdict, Refusal};
use super::types::SecurityPolicy;

/// The answer for a command that will run inside the kernel sandbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JailedVerdict {
    pub verdict: CommandVerdict,
    /// TinySecurity refused the command only for naming a protected path, and
    /// this answer was taken without that floor. The caller must run the
    /// command inside the jail, or refuse it with [`Refusal::Protected`].
    pub floor_waived: bool,
}

impl JailedVerdict {
    /// An answer that waived nothing.
    pub fn checked(verdict: CommandVerdict) -> Self {
        Self {
            verdict,
            floor_waived: false,
        }
    }
}

impl SecurityPolicy {
    /// [`Self::check_command`] for a command that will run inside the kernel
    /// sandbox: the protected-path floor is waived, every other refusal stays.
    /// Only the harness flow is considered; scheduled and legacy execution keep
    /// the floor.
    pub async fn check_jailed_command(
        &self,
        query: CommandQuery<'_>,
    ) -> Result<JailedVerdict, Refusal> {
        match self.check_command(query).await {
            Err(Refusal::Protected) if query.flow == CommandFlow::Harness => {}
            other => return other.map(JailedVerdict::checked),
        }
        let Some(class) = self.protected_command_class(&query).await else {
            return Err(Refusal::Protected);
        };
        let neutral = neutral_command_text(query.command);
        if neutral == query.command {
            return Err(Refusal::Protected);
        }
        let declared = Some(query.declared.map_or(class, |declared| declared.max(class)));
        let verdict = self
            .check_command(CommandQuery {
                command: &neutral,
                declared,
                ..query
            })
            .await?;
        log::debug!(
            "[security][command] protected-path floor waived for a jailed command \
             class={class:?} stage={:?} gate={:?}",
            query.stage,
            verdict.gate
        );
        Ok(JailedVerdict {
            verdict,
            floor_waived: true,
        })
    }
}

/// `command` with every path-like word replaced by `x`, for asking the module
/// about everything but the protected-path floor. A path-like word is a run of
/// word characters holding `.`, `/` or `~`. Quotes, `$`, backticks,
/// parentheses, redirections and separators are kept as they are, so the
/// module still sees any hidden execution. The words that decide the verdict
/// are kept too: a command word keeps its name (`/bin/rm` becomes `rm`,
/// `mkfs.ext4` stays), so the high-risk and allowlist checks still see what
/// runs, and a leading `NAME=value` assignment keeps its name (`NAME=x`). The
/// result is only ever classified, never run.
pub(crate) fn neutral_command_text(command: &str) -> String {
    fn word_char(c: char) -> bool {
        c.is_alphanumeric()
            || matches!(
                c,
                '.' | '/' | '~' | '-' | '_' | '+' | '@' | '%' | ':' | ',' | '='
            )
    }
    fn assignment_name(word: &str) -> Option<&str> {
        let (name, _) = word.split_once('=')?;
        let mut chars = name.chars();
        let first = chars.next()?;
        ((first.is_ascii_alphabetic() || first == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .then_some(name)
    }
    /// Pushes the neutral form of `word`; returns whether the next word is
    /// still in command position.
    fn flush(word: &mut String, out: &mut String, command_position: bool) -> bool {
        let mut next = command_position;
        if word.is_empty() {
            return next;
        }
        let pathlike = word.contains(['.', '/', '~']);
        if command_position {
            if let Some(name) = assignment_name(word) {
                out.push_str(name);
                out.push('=');
                if word[name.len() + 1..].is_empty() || !pathlike {
                    out.push_str(&word[name.len() + 1..]);
                } else {
                    out.push('x');
                }
            } else {
                next = false;
                let base = word.rsplit('/').next().unwrap_or("");
                if !pathlike {
                    out.push_str(word);
                } else if base.is_empty()
                    || base.starts_with(['.', '~'])
                    || (!word.contains('/') && word.starts_with(['.', '~']))
                {
                    out.push('x');
                } else {
                    out.push_str(base);
                }
            }
        } else if pathlike {
            out.push('x');
        } else {
            out.push_str(word);
        }
        word.clear();
        next
    }
    let mut out = String::with_capacity(command.len());
    let mut word = String::new();
    let mut command_position = true;
    for c in command.chars() {
        if word_char(c) {
            word.push(c);
        } else {
            command_position = flush(&mut word, &mut out, command_position);
            if matches!(c, ';' | '&' | '|' | '(' | '\n' | '`') {
                command_position = true;
            }
            out.push(c);
        }
    }
    flush(&mut word, &mut out, command_position);
    out
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "jailed_commands_tests.rs"]
mod tests;
