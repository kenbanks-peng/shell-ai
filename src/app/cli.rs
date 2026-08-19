//! Command-line interface definitions.

use clap::{Parser, Subcommand};

use super::integration::Shell;

#[derive(Parser)]
#[command(
    name = "shell-ai",
    version,
    about = "Return one shell command from an inference provider"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Request a shell command. If omitted, this is the default command.
    Ask { request: Vec<String> },
    /// Open the terminal request UI and print the suggested command.
    Exec,
    /// View or change the persistent provider/model selection.
    Model {
        #[command(subcommand)]
        command: Option<ModelCommand>,
    },
    /// Open the command menu.
    Menu,
    /// Search previously submitted prompts and print the selected prompt.
    History,
    /// Print the path to the configuration file.
    ConfigPath,
    /// Create the configuration file.
    Install,
    /// Check the local installation and configuration.
    Doctor,
    /// Print shell integration code.
    Init { shell: Shell },
}

#[derive(Subcommand)]
pub enum ModelCommand {
    Show,
    List,
    Select,
    Use { selection: String },
}
