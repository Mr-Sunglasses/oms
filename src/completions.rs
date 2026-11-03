//! `oms completions <shell>`: tab completion for commands and theme names.

use anyhow::{Result, bail};

/// Every command a user types. A test in main.rs checks this against `--help`.
pub const COMMANDS: &[&str] = &[
    "apply",
    "list",
    "status",
    "auto",
    "rotate",
    "wallpapers",
    "apps",
    "config",
    "update",
    "self-update",
    "doctor",
    "uninstall",
    "completions",
];

const ZSH: &str = r#"#compdef oms
# oms completions for zsh. Install: oms completions zsh > "${fpath[1]}/_oms"
_oms() {
  local -a commands themes
  commands=(OMS_COMMANDS)
  if (( CURRENT == 2 )); then
    _describe 'command' commands
    return
  fi
  themes=(${(f)"$(oms list --names 2>/dev/null)"})
  case ${words[2]} in
    apply|auto) _describe 'theme' themes ;;
    wallpapers)
      if (( CURRENT == 3 )); then _values 'action' add remove list
      elif (( CURRENT == 4 )); then _describe 'theme' themes
      else _files; fi ;;
    apps)
      if (( CURRENT == 3 )); then _values 'action' on off
      else _values 'app' OMS_APPS; fi ;;
    config) (( CURRENT == 3 )) && _values 'action' install show sections restore ;;
    rotate) _values 'interval' 15m 30m 1h 2h 1d off ;;
    completions) _values 'shell' zsh bash fish ;;
  esac
}
# Autoloaded from $fpath (e.g. by Homebrew): complete now. Sourced: register.
if [[ $funcstack[1] == _oms ]]; then
  _oms "$@"
else
  compdef _oms oms
fi
"#;

const BASH: &str = r#"# oms completions for bash. Install: oms completions bash > ~/.local/share/bash-completion/completions/oms
_oms() {
  local cur=${COMP_WORDS[COMP_CWORD]} cmd=${COMP_WORDS[1]}
  if (( COMP_CWORD == 1 )); then
    COMPREPLY=($(compgen -W "OMS_COMMANDS" -- "$cur")); return
  fi
  case $cmd in
    apply|auto) COMPREPLY=($(compgen -W "$(oms list --names 2>/dev/null)" -- "$cur")) ;;
    wallpapers)
      if (( COMP_CWORD == 2 )); then COMPREPLY=($(compgen -W "add remove list" -- "$cur"))
      elif (( COMP_CWORD == 3 )); then COMPREPLY=($(compgen -W "$(oms list --names 2>/dev/null)" -- "$cur"))
      else COMPREPLY=($(compgen -f -- "$cur")); fi ;;
    apps)
      if (( COMP_CWORD == 2 )); then COMPREPLY=($(compgen -W "on off" -- "$cur"))
      else COMPREPLY=($(compgen -W "OMS_APPS" -- "$cur")); fi ;;
    config) COMPREPLY=($(compgen -W "install show sections restore" -- "$cur")) ;;
    rotate) COMPREPLY=($(compgen -W "15m 30m 1h 2h 1d off" -- "$cur")) ;;
    completions) COMPREPLY=($(compgen -W "zsh bash fish" -- "$cur")) ;;
  esac
}
complete -F _oms oms
"#;

const FISH: &str = r#"# oms completions for fish. Install: oms completions fish > ~/.config/fish/completions/oms.fish
set -l commands OMS_COMMANDS
complete -c oms -f
complete -c oms -n "not __fish_seen_subcommand_from $commands" -a "$commands"
complete -c oms -n "__fish_seen_subcommand_from apply auto" -a "(oms list --names 2>/dev/null)"
complete -c oms -n "__fish_seen_subcommand_from wallpapers" -a "add remove list (oms list --names 2>/dev/null)"
complete -c oms -n "__fish_seen_subcommand_from apps" -a "on off OMS_APPS"
complete -c oms -n "__fish_seen_subcommand_from config" -a "install show sections restore"
complete -c oms -n "__fish_seen_subcommand_from rotate" -a "15m 30m 1h 2h 1d off"
complete -c oms -n "__fish_seen_subcommand_from completions" -a "zsh bash fish"
"#;

pub fn print(shell: Option<&str>) -> Result<()> {
    let template = match shell {
        Some("zsh") => ZSH,
        Some("bash") => BASH,
        Some("fish") => FISH,
        _ => bail!(
            "which shell? Add one of these to your shell's config:\n\n  \
             zsh:   source <(oms completions zsh)\n  \
             bash:  source <(oms completions bash)\n  \
             fish:  oms completions fish | source"
        ),
    };
    print!("{}", script(template));
    Ok(())
}

/// Fills in the command and app lists.
fn script(template: &str) -> String {
    template
        .replace("OMS_COMMANDS", &COMMANDS.join(" "))
        .replace("OMS_APPS", &crate::apps::names().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_name_every_command_and_app() {
        for template in [ZSH, BASH, FISH] {
            let script = script(template);
            assert!(!script.contains("OMS_"), "{script}");
            for word in COMMANDS.iter().chain(crate::apps::names().iter()) {
                assert!(script.contains(word), "{word} missing");
            }
        }
    }
}
