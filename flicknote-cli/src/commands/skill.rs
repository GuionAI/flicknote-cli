use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use flicknote_core::error::CliError;

const FLICKNOTE_SKILL: &str = include_str!("../../../skills/flicknote.md");

#[derive(Args)]
pub(crate) struct SkillArgs {
    #[command(subcommand)]
    pub(crate) command: SkillCommand,
}

#[derive(Subcommand)]
pub(crate) enum SkillCommand {
    /// Install the FlickNote agent skill
    Install,
}

pub(crate) fn run(args: &SkillArgs) -> Result<(), CliError> {
    match args.command {
        SkillCommand::Install => install_default(),
    }
}

fn install_default() -> Result<(), CliError> {
    let home =
        dirs::home_dir().ok_or_else(|| CliError::Other("could not find home directory".into()))?;
    let installed = install_to_homes(&home)?;

    for path in installed {
        println!("Installed {}", path.display());
    }

    Ok(())
}

fn install_to_homes(home: &Path) -> Result<Vec<PathBuf>, CliError> {
    install_to_bases([home.join(".agents").join("skills")])
}

fn install_to_bases<I>(bases: I) -> Result<Vec<PathBuf>, CliError>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut installed = Vec::new();

    for base in bases {
        let skill_dir = base.join("flicknote");
        fs::create_dir_all(&skill_dir)?;
        let path = skill_dir.join("SKILL.md");
        fs::write(&path, FLICKNOTE_SKILL)?;
        installed.push(path);
    }

    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_to_bases_writes_only_the_standard_agent_skill_destination() {
        let temp = tempfile::tempdir().expect("temp dir");
        let agents = temp.path().join(".agents").join("skills");

        let installed = install_to_bases([agents.clone()]).expect("install skill");

        assert_eq!(installed, vec![agents.join("flicknote").join("SKILL.md")]);
        assert_eq!(
            fs::read_to_string(agents.join("flicknote").join("SKILL.md")).expect("agents skill"),
            FLICKNOTE_SKILL
        );
    }

    #[test]
    fn install_to_homes_leaves_existing_claude_skill_untouched() {
        let temp = tempfile::tempdir().expect("temp dir");
        let claude_skill = temp.path().join(".claude/skills/flicknote/SKILL.md");
        fs::create_dir_all(claude_skill.parent().unwrap()).expect("claude skill dir");
        fs::write(&claude_skill, "user-managed skill").expect("existing claude skill");

        install_to_homes(temp.path()).expect("install skill");

        assert_eq!(
            fs::read_to_string(claude_skill).unwrap(),
            "user-managed skill"
        );
        assert!(
            temp.path()
                .join(".agents/skills/flicknote/SKILL.md")
                .exists()
        );
    }
}
