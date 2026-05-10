//! # fbx — FreeBox Command Line Interface
//!
//! A first-class CLI client for the FreeBox platform. Ships as a single
//! statically-linked binary with no runtime dependencies.
//!
//! ## Command Overview
//!
//! ```text
//! fbx auth register    Register a new FreeBox account
//! fbx auth login       Login and store credentials in the OS keychain
//! fbx auth logout      Remove stored credentials
//! fbx auth whoami      Display the currently logged-in user
//!
//! fbx upload <file>    Upload file(s) with E2EE (shows progress bar)
//! fbx download <id>    Download and decrypt a file
//! fbx ls [path]        List files in your FreeBox
//! fbx rm <file>        Delete a file (moves to trash)
//! fbx sync <dir>       Two-way sync a local directory
//!
//! fbx msg send <user>  Send an encrypted message
//! fbx msg read         Read unread messages
//!
//! fbx mail compose     Compose and send encrypted email
//! fbx mail read        Read inbox
//!
//! fbx datasource list  List available and configured storage data sources
//! fbx datasource add   Register a cloud or local storage data source
//! fbx datasource rm    Remove a configured data source
//!
//! fbx plugin install   Install a FreeBox plugin
//! fbx plugin list      List installed plugins
//! fbx plugin rm        Remove a plugin
//! ```

use anyhow::Result;
use clap::{Parser, Subcommand};

mod auth;
mod client;
mod commands;
mod config;
mod session;

// ---------------------------------------------------------------------------
// Top-level CLI structure
// ---------------------------------------------------------------------------

/// FreeBox — Encrypted cloud storage, messaging, and email.
#[derive(Parser)]
#[command(
    name = "fbx",
    version,
    author,
    about = "FreeBox CLI — encrypted cloud storage, messaging, and email",
    long_about = None,
    propagate_version = true,
)]
pub struct Cli {
    /// FreeBox server URL. Overrides config file.
    #[arg(
        long,
        global = true,
        env = "FREEBOX_SERVER",
        default_value = "https://freebox.io"
    )]
    pub server: String,

    /// Suppress all output except errors.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Enable verbose debug output.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Authentication — register, login, logout.
    Auth {
        #[command(subcommand)]
        cmd: AuthCommands,
    },

    /// Upload one or more files (automatically encrypted).
    Upload {
        /// Local file(s) to upload.
        #[arg(required = true)]
        files: Vec<std::path::PathBuf>,
        /// Remote destination path (e.g. `remote://documents/`).
        #[arg(short, long, default_value = "remote://")]
        destination: String,
        /// Number of parallel chunk upload streams (1–16).
        #[arg(short, long, default_value = "8")]
        parallelism: u8,
    },

    /// Download and decrypt a file by its remote path or ID.
    Download {
        /// Remote file path or UUID.
        remote: String,
        /// Local destination path.
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },

    /// List files in your FreeBox (like `ls`).
    #[command(alias = "ls")]
    List {
        /// Remote path prefix to list (default: root).
        #[arg(default_value = "remote://")]
        path: String,
        /// Show full metadata (size, date, content hash).
        #[arg(short, long)]
        long: bool,
    },

    /// Delete a file (soft delete — recoverable from trash for 30 days).
    #[command(alias = "rm")]
    Remove {
        /// Remote file path or UUID.
        remote: String,
        /// Permanently delete (bypass trash).
        #[arg(long)]
        permanent: bool,
    },

    /// Sync a local directory with a remote path.
    Sync {
        /// Local directory to sync.
        local: std::path::PathBuf,
        /// Remote path to sync to/from.
        remote: String,
        /// Watch for changes and sync continuously.
        #[arg(short, long)]
        watch: bool,
        /// Sync direction.
        #[arg(long, default_value = "both", value_enum)]
        direction: SyncDirection,
    },

    /// Encrypted real-time messaging.
    Msg {
        #[command(subcommand)]
        cmd: MsgCommands,
    },

    /// Encrypted email.
    Mail {
        #[command(subcommand)]
        cmd: MailCommands,
    },

    /// Manage storage data sources.
    #[command(visible_alias = "provider")]
    Datasource {
        #[command(subcommand)]
        cmd: ProviderCommands,
    },

    /// Manage storage buckets (list, create).
    Bucket {
        #[command(subcommand)]
        cmd: commands::bucket::BucketCommand,
    },

    /// Manage FreeBox plugins.
    Plugin {
        #[command(subcommand)]
        cmd: PluginCommands,
    },
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AuthCommands {
    /// Create a new FreeBox account.
    Register {
        #[arg(short, long)]
        username: Option<String>,
        #[arg(short, long)]
        email: Option<String>,
    },
    /// Login and store credentials in the OS keychain.
    Login {
        #[arg(short, long)]
        username: Option<String>,
    },
    /// Remove stored credentials.
    Logout,
    /// Display the currently logged-in user.
    Whoami,
}

#[derive(Subcommand)]
pub enum MsgCommands {
    /// Send an encrypted message to a user.
    Send {
        /// Recipient username (e.g. `@alice`).
        to: String,
        /// Message text. Omit for interactive compose.
        message: Option<String>,
    },
    /// Read unread messages.
    Read {
        /// Show messages from a specific user.
        #[arg(short, long)]
        from: Option<String>,
    },
    /// List conversations.
    List,
}

#[derive(Subcommand)]
pub enum MailCommands {
    /// Compose and send an encrypted email.
    Compose {
        #[arg(long)]
        to: Vec<String>,
        #[arg(long)]
        subject: Option<String>,
    },
    /// Read inbox.
    Read,
    /// List inbox.
    List {
        #[arg(long, default_value = "inbox")]
        folder: String,
    },
}

#[derive(Subcommand)]
pub enum ProviderCommands {
    /// Add a storage data source.
    Add {
        /// Data source type: cloudflare-r2, aws-s3, minio, backblaze-b2, local.
        provider: String,
        // Further flags depend on the provider; parsed interactively.
    },
    /// List available and configured data sources.
    List,
    /// Remove a configured data source.
    Remove { provider_id: String },
    /// Test connectivity to a data source.
    Test { provider_id: String },
}

#[derive(Subcommand)]
pub enum PluginCommands {
    /// Install a plugin from the registry or a local path.
    Install {
        /// Plugin ID (e.g. `freebox-calendar`) or path to a local plugin.
        plugin: String,
    },
    /// List installed plugins.
    List,
    /// Remove a plugin.
    Remove { plugin_id: String },
    /// Update all installed plugins.
    Update,
}

#[derive(clap::ValueEnum, Clone)]
pub enum SyncDirection {
    /// Upload local changes to remote.
    Up,
    /// Download remote changes to local.
    Down,
    /// Two-way sync (conflict resolution via CRDT).
    Both,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize a simple logger — no JSON needed for a CLI.
    if cli.verbose {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .init();
    } else if !cli.quiet {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .init();
    }

    // If the user didn't pass --server or FREEBOX_SERVER, fall back to the
    // value from the config file (which itself defaults to https://freebox.io).
    let cfg_server;
    let server = if cli.server == session::DEFAULT_SERVER {
        cfg_server = config::CliConfig::load().server;
        &cfg_server
    } else {
        &cli.server
    };

    match cli.command {
        Commands::Auth { cmd } => auth::handle(cmd, server).await,

        Commands::Upload {
            files,
            destination,
            parallelism,
        } => commands::upload::run(files, destination, parallelism, server).await,

        Commands::Download { remote, output } => {
            commands::download::run(remote, output, server).await
        }

        Commands::List { path, long } => commands::list::run(path, long, server).await,

        Commands::Remove { remote, permanent } => {
            commands::remove::run(remote, permanent, server).await
        }

        Commands::Sync {
            local,
            remote,
            watch,
            direction,
        } => commands::sync::run(local, remote, watch, direction, server).await,

        Commands::Msg { cmd } => commands::msg::handle(cmd, server).await,
        Commands::Mail { cmd } => commands::mail::handle(cmd, server).await,
        Commands::Datasource { cmd } => commands::provider::handle(cmd, server).await,
        Commands::Bucket { cmd } => commands::bucket::handle(cmd, server).await,
        Commands::Plugin { cmd } => commands::plugin::handle(cmd, server).await,
    }
}
