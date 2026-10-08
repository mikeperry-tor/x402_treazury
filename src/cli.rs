//! Public command tree. Inspection commands never enter the serving branch.
use super::Args;
use clap::{Args as ClapArgs, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "x402_treazury",
    version,
    about = "Private x402 API access through MCP",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start MCP servers and configured automatic wallet funding.
    Serve(Serve),
    /// Inspect catalogs or warm discovery caches.
    #[command(subcommand)]
    Catalog(Catalog),
    /// Inspect configuration or validate selected catalogs without funding wallets.
    #[command(subcommand)]
    Config(Configuration),
    /// Initialize, inspect, back up and manage the Zcash treasury.
    Wallet(x402_treazury::wallet_cli::WalletArgs),
    /// Inspect persisted agent-added source records.
    #[command(subcommand)]
    Sources(Sources),
    /// Print build identity and provenance as JSON.
    BuildInfo,
}

#[derive(Subcommand)]
pub enum Catalog {
    /// Warm discovery caches; configured relay may pay. --direct bypasses Tor without relay.
    Warm(Warm),
    /// List selected tools; optionally discover unsigned x402 prices.
    Tools(Tools),
    /// List available OpenAPI tags.
    Tags(Input),
    /// Preview a standalone provider tool's HTTP request without sending it.
    Route(Route),
}

#[derive(Subcommand)]
pub enum Configuration {
    /// Resolve TOML configuration offline without wallets, credentials or catalog I/O.
    Show(Show),
    /// Fetch catalogs and validate selection without starting servers or funding.
    Check(Box<Check>),
}

#[derive(Subcommand)]
pub enum Sources {
    /// Remove a saved source while serving is stopped. Does not affect wallet state.
    Remove {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server: String,
        #[arg(long)]
        source_id: String,
    },
    /// Fetch and validate a saved source's current specification while serving is stopped.
    Refresh {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server: String,
        #[arg(long)]
        source_id: String,
    },
    /// Read persisted agent-added sources without fetching or modifying them.
    Inspect {
        #[arg(long, alias = "meta-config")]
        config: PathBuf,
    },
}

#[derive(ClapArgs, Default)]
pub struct Input {
    /// Deployment configuration. Standalone overrides belong in its TOML instead.
    #[arg(long = "config", alias = "meta-config")]
    meta_config: Option<PathBuf>,
    /// Reusable provider TOML for standalone serving or inspection.
    #[arg(long = "provider")]
    config: Option<String>,
    #[arg(long, conflicts_with = "meta_config")]
    network_config: Option<PathBuf>,
    #[arg(long)]
    spec: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    prefix: Option<String>,
    #[arg(long, value_delimiter = ',')]
    include: Option<Vec<String>>,
    #[arg(long, value_delimiter = ',')]
    exclude: Option<Vec<String>>,
    #[arg(long, value_delimiter = ',')]
    tags: Option<Vec<String>>,
    #[arg(long, value_delimiter = ',')]
    exclude_tags: Option<Vec<String>>,
    #[arg(long)]
    env_file: Option<String>,
    #[arg(long)]
    max_response_bytes: Option<usize>,
    #[arg(long)]
    max_help_bytes: Option<usize>,
    #[arg(long)]
    max_spec_bytes: Option<usize>,
    #[arg(long)]
    timeout: Option<f64>,
}

#[derive(ClapArgs)]
pub struct Serve {
    #[command(flatten)]
    input: Input,
    #[arg(long)]
    max_api_payment_usdc: Option<String>,
    #[arg(long)]
    max_response_chars: Option<usize>,
    #[arg(long, default_value = "stdio", value_parser = ["stdio", "http"])]
    transport: String,
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value = "8000")]
    port: u16,
    #[arg(long)]
    bearer_token: Option<String>,
    /// Explicitly disable authentication for standalone HTTP serving.
    #[arg(long, conflicts_with = "bearer_token")]
    no_auth: bool,
    /// Replace the MCP Host allowlist (comma-separated hosts or host:port entries).
    #[arg(long, value_delimiter = ',', conflicts_with = "disable_host_check")]
    allowed_hosts: Option<Vec<String>>,
    /// Disable the MCP Host allowlist; authentication is unchanged.
    #[arg(long)]
    disable_host_check: bool,
    #[arg(long, hide = true, requires = "meta_config")]
    qualification_no_new_funding: bool,
    #[arg(
        long,
        hide = true,
        requires = "qualification_parent_stdin",
        conflicts_with = "env_file"
    )]
    qualification_unsigned: bool,
    #[arg(long, hide = true, requires = "qualification_binding", requires = "qualification_parent_stdin", conflicts_with_all = ["qualification_unsigned", "env_file"])]
    qualification_managed: bool,
    #[arg(long, hide = true, requires = "meta_config")]
    qualification_parent_stdin: bool,
    #[arg(long, hide = true, requires = "qualification_parent_stdin")]
    qualification_binding: Option<PathBuf>,
}

#[derive(ClapArgs)]
pub struct Warm {
    /// Deployment whose existing treasury state directory and cache policy are used.
    #[arg(long, alias = "meta-config")]
    pub config: PathBuf,
    /// Source IDs to warm (repeat or comma-separate); defaults to all declared sources.
    #[arg(long, value_delimiter = ',')]
    pub source: Vec<String>,
    /// Fetch directly and explicitly permit fresh cached data in this deployment's network policy.
    #[arg(long)]
    pub direct: bool,
    /// Also discover/cache eligible unsigned pricing estimates, respecting listener filters.
    #[arg(long)]
    pub discover_pricing: bool,
}

#[derive(ClapArgs)]
pub struct Tools {
    #[command(flatten)]
    input: Input,
    /// Run unsigned startup pricing discovery; respects source probe opt-outs.
    #[arg(long)]
    discover_pricing: bool,
}

#[derive(ClapArgs)]
pub struct Route {
    #[command(flatten)]
    input: Input,
    /// Generated tool name to preview.
    #[arg(conflicts_with = "meta_config")]
    tool: String,
    /// Tool arguments as a JSON object.
    #[arg(long, default_value = "{}")]
    args: String,
}

#[derive(ClapArgs)]
#[group(required = true, multiple = false)]
pub struct Location {
    #[arg(long = "config", alias = "meta-config")]
    meta_config: Option<PathBuf>,
    #[arg(long = "provider")]
    config: Option<String>,
}

#[derive(ClapArgs)]
pub struct Show {
    #[command(flatten)]
    location: Location,
    #[arg(long, conflicts_with = "meta_config")]
    network_config: Option<PathBuf>,
}

#[derive(ClapArgs)]
pub struct Check {
    #[command(flatten)]
    input: Input,
    #[arg(long, hide = true, requires = "meta_config")]
    qualification_snapshot: bool,
}

impl From<Input> for Args {
    fn from(input: Input) -> Self {
        let Input {
            meta_config,
            config,
            network_config,
            spec,
            base_url,
            prefix,
            include,
            exclude,
            tags,
            exclude_tags,
            env_file,
            max_response_bytes,
            max_help_bytes,
            max_spec_bytes,
            timeout,
        } = input;
        Self {
            meta_config,
            config,
            network_config,
            spec,
            base_url,
            prefix,
            include,
            exclude,
            tags,
            exclude_tags,
            env_file,
            max_response_bytes,
            max_help_bytes,
            max_spec_bytes,
            timeout,
            ..Default::default()
        }
    }
}

impl From<Serve> for Args {
    fn from(serve: Serve) -> Self {
        let Serve {
            input,
            max_api_payment_usdc,
            max_response_chars,
            transport,
            host,
            port,
            bearer_token,
            no_auth,
            allowed_hosts,
            disable_host_check,
            qualification_no_new_funding,
            qualification_unsigned,
            qualification_managed,
            qualification_parent_stdin,
            qualification_binding,
        } = serve;
        Self {
            max_api_payment_usdc,
            max_response_chars,
            transport,
            host,
            port,
            bearer_token,
            no_auth,
            allowed_hosts,
            disable_host_check,
            qualification_no_new_funding,
            qualification_unsigned,
            qualification_managed,
            qualification_parent_stdin,
            qualification_binding,
            ..input.into()
        }
    }
}

impl From<Catalog> for Args {
    fn from(command: Catalog) -> Self {
        match command {
            Catalog::Warm(_) => unreachable!("cache warming is dispatched separately"),
            Catalog::Tools(tools) => Self {
                list_tools: true,
                discover_pricing: tools.discover_pricing,
                ..tools.input.into()
            },
            Catalog::Tags(input) => Self {
                list_tags: true,
                ..input.into()
            },
            Catalog::Route(route) => Self {
                route_tool: Some(route.tool),
                args: route.args,
                ..route.input.into()
            },
        }
    }
}

impl From<Configuration> for Args {
    fn from(command: Configuration) -> Self {
        match command {
            Configuration::Show(show) => Self {
                show_config: true,
                meta_config: show.location.meta_config,
                config: show.location.config,
                network_config: show.network_config,
                ..Default::default()
            },
            Configuration::Check(check) => Self {
                check: true,
                qualification_snapshot: check.qualification_snapshot,
                ..check.input.into()
            },
        }
    }
}
