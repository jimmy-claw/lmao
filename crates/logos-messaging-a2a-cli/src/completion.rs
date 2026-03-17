use clap::CommandFactory;
use clap_complete::{generate, Shell};

use crate::cli::Cli;

/// Print shell completions for the given shell to stdout.
pub fn handle(shell: Shell) {
    let mut cmd = Cli::command();
    generate(
        shell,
        &mut cmd,
        "logos-messaging-a2a",
        &mut std::io::stdout(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generates_non_empty(shell: Shell) {
        let mut cmd = Cli::command();
        let mut buf = Vec::new();
        generate(shell, &mut cmd, "logos-messaging-a2a", &mut buf);
        assert!(
            !buf.is_empty(),
            "completion output for {shell:?} should not be empty"
        );
    }

    #[test]
    fn bash_completions() {
        generates_non_empty(Shell::Bash);
    }

    #[test]
    fn zsh_completions() {
        generates_non_empty(Shell::Zsh);
    }

    #[test]
    fn fish_completions() {
        generates_non_empty(Shell::Fish);
    }

    #[test]
    fn powershell_completions() {
        generates_non_empty(Shell::PowerShell);
    }

    #[test]
    fn elvish_completions() {
        generates_non_empty(Shell::Elvish);
    }
}
